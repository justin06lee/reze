import RezeKit
import SwiftUI

// The keyboard: the one thing on a phone that may type into another app.
// It shows the macros the app last fetched, and types the one that is tapped;
// and the clipboard, the phone's and the Mac's, to type from as well.
//
// Everything it does is here, where it builds and can be read on a Mac; the
// extension itself (`ios/Keyboard`) is only the view controller iOS asks for,
// handing this the text field it is typing into.

@MainActor
public final class KeyboardState: ObservableObject {
    /// The text field being typed into, as iOS lets a keyboard see it.
    public struct Proxy {
        public var insert: (String) -> Void
        public var deleteBackward: () -> Void
        public var nextKeyboard: () -> Void
        /// The text before the caret — as much of it as iOS hands over,
        /// which is about a sentence.
        public var context: () -> String
        /// Whether the keyboard was given Full Access in Settings. Without
        /// it, iOS keeps a keyboard away from the clipboard.
        public var fullAccess: () -> Bool
        /// Whether to draw a globe key: not on phones that draw their own
        /// under the keyboard.
        public var needsGlobe: () -> Bool

        public init(
            insert: @escaping (String) -> Void,
            deleteBackward: @escaping () -> Void,
            nextKeyboard: @escaping () -> Void,
            context: @escaping () -> String,
            fullAccess: @escaping () -> Bool,
            needsGlobe: @escaping () -> Bool
        ) {
            self.insert = insert
            self.deleteBackward = deleteBackward
            self.nextKeyboard = nextKeyboard
            self.context = context
            self.fullAccess = fullAccess
            self.needsGlobe = needsGlobe
        }
    }

    /// The biggest attachment the keyboard will put on the clipboard. A file
    /// is read into memory to be copied, and a keyboard that uses too much is
    /// closed by iOS mid-word; the app has no such limit.
    public static let largest: Int64 = 25_000_000

    @Published public private(set) var library: Library?
    @Published public private(set) var context = ""
    @Published public private(set) var fullAccess = false
    @Published public private(set) var needsGlobe = true
    @Published public var notice: String?
    @Published var tag: String?
    /// Whether the keys are the clipboard list rather than the macros.
    @Published var showingClips = false
    @Published public private(set) var clips: [ClipEntry] = []
    /// Why the clipboard could not be evened out with the Mac, if it could not.
    @Published public private(set) var clipTrouble: String?

    public let shelf: Shelf?
    private let proxy: Proxy
    /// The phone's clipboard, or `nil` to leave it and the network alone.
    private let pasteboard: Pasteboard?
    private let connect: (Mac) -> Client
    private var exchanging = false
    private var lastAsked = Date.distantPast
    private var wait: TimeInterval = 8

    public init(
        shelf: Shelf? = .shared(),
        proxy: Proxy,
        pasteboard: Pasteboard? = nil,
        connect: @escaping (Mac) -> Client = { Client($0) }
    ) {
        self.shelf = shelf
        self.proxy = proxy
        self.pasteboard = pasteboard
        self.connect = connect
    }

    /// Read the library again: the app may have fetched since the keyboard
    /// was last up.
    public func reload() {
        library = shelf?.library()
        clips = shelf?.clips() ?? []
        if let tag, !tags.contains(tag) { self.tag = nil }
        lastAsked = .distantPast
        look()
    }

    /// Called every second or two while the keyboard is up: notice a fresh
    /// copy on the phone, and now and then ask the Mac for its newest.
    ///
    /// Only with Full Access — without it iOS gives a keyboard neither the
    /// clipboard nor the network. This is the closest a phone comes to
    /// watching its clipboard: a keyboard is up for most of what is copied.
    public func tick(now: Date = Date()) async {
        guard let shelf, let pasteboard, proxy.fullAccess(), !exchanging,
              let mac = shelf.fetched()?.mac
        else { return }
        // An image is taken only as the PNG or JPEG it was copied as:
        // redrawing one takes more memory than a keyboard is allowed.
        let copied = pasteboard.fresh(for: shelf, decoding: false)
        guard copied != nil || now.timeIntervalSince(lastAsked) >= wait else { return }
        exchanging = true
        defer { exchanging = false }
        lastAsked = now
        let result = await shelf.exchange(with: connect(mac), copied: copied, name: Clipboard.deviceName)
        clips = shelf.clips()
        clipTrouble = result.trouble
        // A Mac that is asleep takes seconds to not answer; ask it less.
        wait = result.sharing == nil ? 30 : 8
        if let sharing = result.sharing, var known = shelf.fetched(), known.clipboard != sharing {
            known.clipboard = sharing
            try? shelf.keep(known)
        }
    }

    /// Type a remembered copy, or for an image put it back on the clipboard.
    func use(_ clip: ClipEntry) {
        if clip.isImage {
            guard fullAccess else {
                notice = "Pictures need Full Access: Settings → General → Keyboard → Keyboards → Reze"
                return
            }
            guard let shelf, clip.bytes <= KeyboardState.largest, Clipboard.set(clip, in: shelf) else {
                notice = "Too big for a keyboard to copy. Copy it from the Reze app."
                return
            }
            if let pasteboard { shelf.sawPasteboard(pasteboard.count()) }
            notice = "Copied the image. Tap the text field, then Paste."
        } else {
            proxy.insert(clip.text)
        }
        look()
    }

    /// What the clipboard list says above itself, when something is in the way.
    var clipNote: String? {
        if !fullAccess { return "Turn on Allow Full Access for this keyboard to pick up what you copy and to reach your Mac." }
        if let clipTrouble { return clipTrouble }
        if shelf?.fetched()?.clipboard == false {
            return "Your Mac is not sharing its clipboard: Reze → Settings → Clipboard history, on the Mac."
        }
        return nil
    }

    /// Look at the text field again. Called whenever it changes.
    public func look() {
        context = proxy.context()
        fullAccess = proxy.fullAccess()
        needsGlobe = proxy.needsGlobe()
    }

    /// The macro whose name was just typed, if one was.
    var trigger: Trigger? {
        library.flatMap { Triggers.match(context, in: $0.macros) }
    }

    var tags: [String] {
        var seen = Set<String>()
        return (library?.macros ?? []).flatMap(\.tags).filter { seen.insert($0).inserted }.sorted()
    }

    /// The macros on offer: the most used first, as the palette opens.
    var macros: [Macro] {
        let all = Fuzzy.search("", in: library?.macros ?? [])
        guard let tag else { return all }
        return all.filter { $0.tags.contains(tag) }
    }

    /// Type `macro`. If its name was just typed, the name goes first.
    func type(_ macro: Macro, replacing trigger: Trigger? = nil) {
        guard let library else { return }
        let prepared = Prepared(macro, in: library)
        let pasted = prepared.wantsClipboard && fullAccess ? (Clipboard.text() ?? "") : ""
        // There is nowhere in a keyboard to ask for a variable — its own keys
        // are all it has to type with — so each takes its default.
        let text = prepared.render(clipboard: pasted)

        if let trigger {
            for _ in 0..<trigger.delete { proxy.deleteBackward() }
        }
        if !text.isEmpty {
            proxy.insert(text + (trigger?.trailing ?? ""))
        }

        var said: String?
        if prepared.wantsClipboard, !fullAccess {
            said = "{{clipboard}} was left empty: that needs Full Access"
        } else if prepared.variables.contains(where: \.fallback.isEmpty) {
            said = "Typed with its blanks empty. Open Reze to fill them in."
        }
        if !macro.attachments.isEmpty {
            said = copyAttachments(of: macro)
        } else if text.isEmpty {
            said = "\u{201C}\(macro.name)\u{201D} is empty"
        }
        notice = said
        look()
    }

    /// A keyboard cannot type a picture. It can put one on the clipboard, for
    /// the person to paste. Returns what to tell them.
    private func copyAttachments(of macro: Macro) -> String {
        guard fullAccess else {
            return "Pictures need Full Access: Settings → General → Keyboard → Keyboards → Reze"
        }
        guard let shelf else { return "Open Reze once to fetch your macros" }
        let here = macro.attachments.filter { shelf.has($0) }
        let small = here.filter { $0.bytes <= KeyboardState.largest }
        let files = small.compactMap { attachment in
            shelf.url(for: attachment).map { Clipboard.File(attachment, at: $0) }
        }
        let went = files.isEmpty ? 0 : Clipboard.set(text: nil, files: files)

        if went == 0 {
            return here.isEmpty
                ? "Not fetched yet. Open Reze to fetch it."
                : "Too big for a keyboard to copy. Copy it from the Reze app."
        }
        let what = went == 1 ? small[0].name : "\(went) attachments"
        return "Copied \(what). Tap the text field, then Paste."
    }

    func space() { proxy.insert(" "); look() }
    func newline() { proxy.insert("\n"); look() }
    func delete() { proxy.deleteBackward(); look() }
    func next() { proxy.nextKeyboard() }
}

public struct KeyboardPanel: View {
    @ObservedObject private var state: KeyboardState
    @Environment(\.colorScheme) private var scheme

    public init(state: KeyboardState) {
        self.state = state
    }

    public var body: some View {
        VStack(spacing: 6) {
            if state.showingClips {
                clipList
            } else {
                top
                if state.library?.macros.isEmpty ?? true {
                    empty
                } else {
                    grid
                }
            }
            keys
        }
        .padding(.horizontal, 6)
        .padding(.top, 6)
        .padding(.bottom, 4)
        .tint(Theme.accent(scheme))
        // For as long as the keyboard is up.
        .task {
            while !Task.isCancelled {
                await state.tick()
                try? await Task.sleep(for: .seconds(1.5))
            }
        }
    }

    // The clipboard, in place of the macros: what was copied on this phone
    // and on the Mac. Tap a text to type it.
    @ViewBuilder private var clipList: some View {
        if let notice = state.notice ?? state.clipNote {
            Text(notice)
                .font(.caption)
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity, minHeight: 30)
                .padding(.horizontal, 8)
                .background(Theme.accent(scheme).opacity(0.18), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .accessibilityIdentifier("reze.notice")
                .task(id: state.notice) {
                    guard state.notice != nil else { return }
                    try? await Task.sleep(for: .seconds(4))
                    if !Task.isCancelled { state.notice = nil }
                }
        }
        if state.clips.isEmpty {
            VStack(spacing: 6) {
                Image(systemName: "doc.on.clipboard")
                    .font(.title2)
                    .foregroundStyle(.secondary)
                Text("Nothing copied yet")
                    .font(.subheadline.weight(.medium))
                Text("What you copy here and on your Mac shows up in this list.")
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .multilineTextAlignment(.center)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
            .padding(.horizontal, 24)
        } else {
            ScrollView {
                LazyVStack(spacing: 6) {
                    ForEach(state.clips) { clip in
                        Button {
                            state.use(clip)
                        } label: {
                            ClipRow(clip: clip, shelf: state.shelf, compact: true)
                                .padding(.horizontal, 10)
                                .padding(.vertical, 6)
                                .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                                .background(key, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                        }
                        .buttonStyle(.plain)
                        .accessibilityHint(clip.isImage ? "Copies it" : "Types it")
                    }
                }
            }
            .frame(maxHeight: .infinity)
        }
    }

    // What the strip above the macros says: what just happened, or the macro
    // whose name was just typed, or the tags to narrow by.
    @ViewBuilder private var top: some View {
        if let notice = state.notice {
            Text(notice)
                .font(.footnote.weight(.medium))
                .lineLimit(2)
                .multilineTextAlignment(.center)
                .frame(maxWidth: .infinity, minHeight: 34)
                .padding(.horizontal, 8)
                .background(Theme.accent(scheme).opacity(0.18), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                .accessibilityIdentifier("reze.notice")
                .task(id: notice) {
                    try? await Task.sleep(for: .seconds(4))
                    if !Task.isCancelled { state.notice = nil }
                }
        } else if let trigger = state.trigger {
            Button {
                state.type(trigger.macro, replacing: trigger)
            } label: {
                HStack(spacing: 8) {
                    Image(systemName: "arrow.left.arrow.right")
                    Text("Replace \u{201C}\(trigger.macro.name)\u{201D}")
                        .lineLimit(1)
                }
                .font(.subheadline.weight(.semibold))
                .frame(maxWidth: .infinity, minHeight: 34)
                .foregroundStyle(Theme.onAccent(scheme))
                .background(Theme.accent(scheme), in: RoundedRectangle(cornerRadius: 9, style: .continuous))
            }
            .buttonStyle(.plain)
            .accessibilityIdentifier("reze.replace")
        } else if !state.tags.isEmpty {
            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 6) {
                    chip("All", on: state.tag == nil) { state.tag = nil }
                    ForEach(state.tags, id: \.self) { tag in
                        chip(tag, on: state.tag == tag) { state.tag = state.tag == tag ? nil : tag }
                    }
                }
            }
            .frame(height: 34)
        }
    }

    private func chip(_ title: String, on: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.footnote.weight(.medium))
                .padding(.horizontal, 12)
                .frame(height: 30)
                .foregroundStyle(on ? Theme.onAccent(scheme) : Color.primary)
                .background(on ? Theme.accent(scheme) : key, in: Capsule())
        }
        .buttonStyle(.plain)
    }

    private var grid: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 150), spacing: 6)], spacing: 6) {
                ForEach(state.macros) { macro in
                    Button {
                        state.type(macro)
                    } label: {
                        HStack(spacing: 8) {
                            MacroBadge(macro: macro, shelf: state.shelf, side: 32)
                            Text(macro.name.isEmpty ? "Untitled" : macro.name)
                                .font(.subheadline)
                                .lineLimit(2)
                                .multilineTextAlignment(.leading)
                            Spacer(minLength: 0)
                        }
                        .padding(6)
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        .background(key, in: RoundedRectangle(cornerRadius: 9, style: .continuous))
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityHint(macro.attachments.isEmpty ? "Types it" : "Copies its attachment")
                    .accessibilityIdentifier("reze.key")
                }
            }
        }
        .frame(maxHeight: .infinity)
    }

    private var empty: some View {
        VStack(spacing: 6) {
            Image(systemName: "text.badge.plus")
                .font(.title2)
                .foregroundStyle(.secondary)
            Text("No macros here yet")
                .font(.subheadline.weight(.medium))
            // Nothing readable at all, and no Full Access: the app may well
            // have fetched, and iOS be keeping this keyboard from its copy.
            Text(state.library == nil && !state.fullAccess
                ? "Open the Reze app once to fetch them from your Mac. If you have, turn on Allow Full Access for this keyboard in Settings."
                : "Open the Reze app once to fetch them from your Mac.")
                .font(.footnote)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
        .padding(.horizontal, 24)
    }

    // The few keys a keyboard cannot go without: a way out, a space, a way
    // to take back, and return.
    private var keys: some View {
        HStack(spacing: 6) {
            if state.needsGlobe {
                Button(action: state.next) {
                    Image(systemName: "globe").frame(width: 44, height: 42)
                }
                .background(key, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .accessibilityLabel("Next keyboard")
            }
            Button {
                state.showingClips.toggle()
            } label: {
                Image(systemName: state.showingClips ? "text.badge.plus" : "doc.on.clipboard")
                    .frame(width: 52, height: 42)
            }
            .background(state.showingClips ? Theme.accent(scheme).opacity(0.35) : key, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityLabel(state.showingClips ? "Macros" : "Clipboard")
            Button(action: state.space) {
                Text("space").font(.callout).frame(maxWidth: .infinity, minHeight: 42)
            }
            .background(key, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            Button(action: state.delete) {
                Image(systemName: "delete.left").frame(width: 52, height: 42)
            }
            .buttonRepeatBehavior(.enabled)
            .background(key, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityLabel("Delete")
            Button(action: state.newline) {
                Image(systemName: "return").frame(width: 64, height: 42)
            }
            .background(key, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .accessibilityLabel("Return")
        }
        .buttonStyle(.plain)
        .foregroundStyle(.primary)
    }

    /// The colour of a key, against the keyboard's own backdrop.
    private var key: Color {
        scheme == .dark ? Color(white: 1, opacity: 0.16) : Color(white: 1, opacity: 0.9)
    }
}
