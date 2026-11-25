import Foundation
import RezeKit
import SwiftUI

/// Everything the app's screens show and do: the library as last fetched,
/// which Mac it came from, and the three things there are to do with it —
/// look at the Mac again, copy a macro, take the Mac's clipboard.
@MainActor
public final class Phone: ObservableObject {
    public enum Busy: Equatable {
        case connecting
        /// Looking again; how many attachments are in, of how many.
        case refreshing(done: Int, of: Int)
    }

    @Published public private(set) var library: Library?
    @Published public private(set) var fetched: Fetched?
    @Published public private(set) var busy: Busy?
    /// Why the last look at the Mac came to nothing, for as long as that is true.
    @Published public private(set) var trouble: String?
    /// What just happened, shown for a moment.
    @Published public var notice: String?

    /// What has been copied, here and on the Mac, newest first.
    @Published public private(set) var clips: [ClipEntry] = []
    /// Why the clipboard could not be evened out with the Mac last time.
    @Published public private(set) var clipTrouble: String?

    /// Where the library is kept. `nil` if the build has no App Group, which
    /// is a mistake in how it was signed and is said rather than hidden.
    public let shelf: Shelf?

    /// For talking to a pretend Mac in tests and previews.
    let connect: (Mac) -> Client
    /// The phone's clipboard, or `nil` to leave it alone (tests, drawings).
    let pasteboard: Pasteboard?
    private var exchanging = false

    public init(
        shelf: Shelf? = .shared(),
        pasteboard: Pasteboard? = nil,
        connect: @escaping (Mac) -> Client = { Client($0) }
    ) {
        self.shelf = shelf
        self.pasteboard = pasteboard
        self.connect = connect
        library = shelf?.library()
        fetched = shelf?.fetched()
        clips = shelf?.clips() ?? []
    }

    public var macros: [Macro] { library?.macros ?? [] }

    /// The first time: find the Mac that was typed in, and fetch from it.
    /// Returns whether there is now a Mac.
    @discardableResult
    public func find(_ typed: String) async -> Bool {
        guard let shelf else {
            trouble = "This build has nowhere to keep a library (no App Group). Build it again from the project `make ios` writes."
            return false
        }
        guard let mac = Mac(typed: typed) else {
            trouble = "That does not read as a machine\u{2019}s name or address. Try the name Tailscale shows for the Mac, like mac-mini, or its 100.x.y.z address."
            return false
        }
        busy = .connecting
        defer { busy = nil }
        return await look(at: mac, shelf: shelf)
    }

    /// Look at the Mac again and make the phone's copy the same as its library.
    public func refresh() async {
        guard let shelf, let mac = fetched?.mac, busy == nil else { return }
        busy = .refreshing(done: 0, of: 0)
        defer { busy = nil }
        await look(at: mac, shelf: shelf)
    }

    @discardableResult
    private func look(at mac: Mac, shelf: Shelf) async -> Bool {
        do {
            let result = try await shelf.refresh(from: connect(mac)) { [weak self] done, of in
                Task { @MainActor in
                    guard let self, case .refreshing = self.busy else { return }
                    self.busy = .refreshing(done: done, of: of)
                }
            }
            library = shelf.library()
            fetched = shelf.fetched()
            trouble = nil
            if !result.failed.isEmpty {
                notice = "\(result.failed.count) attachment\(result.failed.count == 1 ? "" : "s") did not arrive. They will be asked for again next time."
            } else if result.changed {
                notice = "\(result.macros) macro\(result.macros == 1 ? "" : "s") from \(mac.label)"
            }
            return true
        } catch is CancellationError {
            return false
        } catch {
            trouble = error.localizedDescription
            return false
        }
    }

    /// Forget the Mac and everything fetched from it.
    public func forget() {
        shelf?.empty()
        library = nil
        fetched = nil
        trouble = nil
        clips = []
        clipTrouble = nil
    }

    /// The attachments of `macro` whose files are on the phone.
    public func files(of macro: Macro) -> [Clipboard.File] {
        guard let shelf else { return [] }
        return macro.attachments.compactMap { attachment in
            guard shelf.has(attachment), let url = shelf.url(for: attachment) else { return nil }
            return Clipboard.File(attachment, at: url)
        }
    }

    /// Copy `macro` — its text with `values` filled in, and its attachments —
    /// so it can be pasted wherever it is wanted.
    public func copy(_ macro: Macro, values: [String: String] = [:]) {
        guard let library else { return }
        let prepared = Prepared(macro, in: library)
        let pasted = prepared.wantsClipboard ? (Clipboard.text() ?? "") : ""
        let text = prepared.render(values: values, clipboard: pasted)
        let files = files(of: macro)
        // Nothing to copy is not a reason to empty the clipboard.
        guard !text.isEmpty || !files.isEmpty else {
            notice = macro.attachments.isEmpty
                ? "\u{201C}\(macro.name)\u{201D} is empty"
                : "Its attachment has not been fetched from the Mac yet"
            return
        }
        let went = Clipboard.set(text: text, files: files)

        let absent = macro.attachments.count - went
        if absent > 0 {
            notice = "Copied, without \(absent) attachment\(absent == 1 ? "" : "s") not fetched yet"
        } else if went > 0, text.isEmpty {
            notice = went == 1 ? "Copied \(macro.attachments[0].name)" : "Copied \(went) attachments"
        } else if went > 0 {
            notice = "Copied, with \(went) attachment\(went == 1 ? "" : "s")"
        } else {
            notice = "Copied"
        }
    }

    /// Whether the Mac shares its clipboard, as of the last time it was asked.
    public var sharing: Bool { fetched?.clipboard == true }

    /// Bring the clipboard list up to date: what is on the phone's clipboard
    /// now, if that has changed, and what was last copied on the Mac — and
    /// hand the Mac the phone's newest. Called each time the app comes to the
    /// front, since a phone lets nothing do this in the background.
    public func exchangeClipboard() async {
        guard let shelf, let mac = fetched?.mac, !exchanging else { return }
        exchanging = true
        defer { exchanging = false }
        let copied = pasteboard?.fresh(for: shelf, decoding: true)
        let result = await shelf.exchange(with: connect(mac), copied: copied, name: Clipboard.deviceName)
        clips = shelf.clips()
        clipTrouble = result.trouble
        if let sharing = result.sharing, var known = fetched, known.clipboard != sharing {
            known.clipboard = sharing
            try? shelf.keep(known)
            fetched = known
        }
    }

    /// Put a remembered copy back on the phone's clipboard.
    public func copy(_ clip: ClipEntry) {
        guard let shelf else { return }
        if Clipboard.set(clip, in: shelf) {
            // Reze put it there, so it is not news the next time it looks.
            if let pasteboard { shelf.sawPasteboard(pasteboard.count()) }
            notice = clip.isImage ? "Copied the image" : "Copied"
        } else {
            notice = "That image is no longer on this phone"
        }
    }

    public func clearClips() {
        shelf?.clearClips()
        clips = []
    }
}
