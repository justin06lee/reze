// Draws the phone's screens to PNG files on a Mac, without a phone, a
// simulator or a window on screen:
//
//   swift run Shots <folder> [mac-name-or-address]
//
// With a Mac named, the screens show its real library (read only); without,
// a small made-up one. What comes out is SwiftUI drawing for macOS at a
// phone's size — the layout, the text and the colours are the app's, the
// system chrome (navigation bars, the search field, list styling) is not, so
// this shows whether a screen holds together, not exactly how iOS draws it.

#if os(macOS)
import AppKit
import RezeKit
@testable import RezeUI
import SwiftUI

let arguments = CommandLine.arguments.dropFirst()
guard let folder = arguments.first else {
    print("usage: swift run Shots <folder> [mac-name-or-address]")
    exit(2)
}
let out = URL(fileURLWithPath: folder, isDirectory: true)
try FileManager.default.createDirectory(at: out, withIntermediateDirectories: true)

let app = NSApplication.shared
app.setActivationPolicy(.prohibited)

@MainActor
func shoot(_ view: some View, _ name: String, size: CGSize, scheme: ColorScheme) {
    let appearance = NSAppearance(named: scheme == .dark ? .darkAqua : .aqua)
    let backdrop = scheme == .dark ? Color(white: 0.09) : Color(white: 0.96)
    let hosting = NSHostingView(
        rootView: view
            .frame(width: size.width, height: size.height)
            .background(backdrop)
            .environment(\.colorScheme, scheme)
    )
    let window = NSWindow(
        contentRect: NSRect(origin: .zero, size: size),
        styleMask: [.borderless],
        backing: .buffered,
        defer: false
    )
    window.appearance = appearance
    window.contentView = hosting
    hosting.layoutSubtreeIfNeeded()
    // Long enough for thumbnails to be read and drawn.
    RunLoop.main.run(until: Date().addingTimeInterval(1.2))
    hosting.layoutSubtreeIfNeeded()

    guard let bitmap = hosting.bitmapImageRepForCachingDisplay(in: hosting.bounds) else { return }
    hosting.cacheDisplay(in: hosting.bounds, to: bitmap)
    let file = out.appendingPathComponent("\(name)-\(scheme == .dark ? "dark" : "light").png")
    try? bitmap.representation(using: .png, properties: [:])?.write(to: file)
    print("drew \(file.lastPathComponent)")
}

let scratch = FileManager.default.temporaryDirectory.appendingPathComponent("reze-shots-\(UUID().uuidString)")
defer { try? FileManager.default.removeItem(at: scratch) }
let shelf = Shelf(root: scratch)

if let typed = arguments.dropFirst().first, let mac = Mac(typed: typed) {
    print("fetching from \(mac.host)…")
    // Waited for by turning the run loop, not with `await`: this file must
    // stay synchronous, or everything after would run inside one main-queue
    // job and the screens' own work (reading thumbnails) could never start.
    final class Waiting: @unchecked Sendable { var result: Result<Refreshed, Error>? }
    let waiting = Waiting()
    Task.detached {
        let result: Result<Refreshed, Error>
        do { result = .success(try await shelf.refresh(from: Client(mac))) } catch { result = .failure(error) }
        DispatchQueue.main.async { waiting.result = result }
    }
    while waiting.result == nil { RunLoop.main.run(until: Date().addingTimeInterval(0.05)) }
    if case .failure(let error) = waiting.result {
        print(error.localizedDescription)
        exit(1)
    }
} else {
    try shelf.keep(Library(macros: [
        Macro(name: "full analysis", description: "The long one", tags: ["prompts"], body: "Do a full analysis of {{topic}}.", usageCount: 9),
        Macro(name: "commit message", tags: ["prompts", "git"], body: "Write a commit message for:\n{{clipboard}}", usageCount: 5),
        Macro(name: "sign off", tags: ["mail"], body: "Thanks,\n{{name|Justin}}", usageCount: 3),
        Macro(name: "address", body: "1 Infinite Loop, Cupertino", usageCount: 2),
        Macro(name: "reaction", tags: ["fun"], attachments: [Attachment(file: "none.png", name: "reaction.png", kind: .image, mime: "image/png", bytes: 482_113)]),
        Macro(name: "demo reel", tags: ["fun"], attachments: [Attachment(file: "none.mp4", name: "reel.mp4", kind: .video, mime: "video/mp4", bytes: 3_370_611)]),
    ]))
    try shelf.keep(Fetched(mac: Mac(host: "mac-mini"), revision: "sample", clipboard: true))
}

let phoneSize = CGSize(width: 393, height: 852)
let keyboardSize = CGSize(width: 393, height: 291)

// Top-level code with no `await` in it is not on the main actor as far as the
// compiler knows, though it is on the main thread; this says so.
MainActor.assumeIsolated {
    for scheme in [ColorScheme.dark, .light] {
        let empty = Phone(shelf: Shelf(root: scratch.appendingPathComponent("nothing")))
        shoot(ConnectView().environmentObject(empty).tint(Theme.accent(scheme)), "connect", size: phoneSize, scheme: scheme)

        let phone = Phone(shelf: shelf)
        shoot(MacrosView().environmentObject(phone).tint(Theme.accent(scheme)), "macros", size: phoneSize, scheme: scheme)
        shoot(SettingsView().environmentObject(phone).tint(Theme.accent(scheme)), "settings", size: phoneSize, scheme: scheme)
        if let asks = phone.macros.first(where: { !Prepared($0, in: phone.library!).variables.isEmpty }) {
            shoot(FillView(macro: asks).environmentObject(phone).tint(Theme.accent(scheme)), "fill", size: phoneSize, scheme: scheme)
        }

        var typed = "please do a "
        let proxy = KeyboardState.Proxy(
            insert: { typed += $0 },
            deleteBackward: { _ = typed.popLast() },
            nextKeyboard: {},
            context: { typed },
            fullAccess: { true },
            needsGlobe: { true }
        )
        let keyboard = KeyboardState(shelf: shelf, proxy: proxy)
        keyboard.reload()
        shoot(KeyboardPanel(state: keyboard), "keyboard", size: keyboardSize, scheme: scheme)

        if let named = phone.macros.first(where: { !$0.name.isEmpty }) {
            typed = "please do a \(named.name)"
            keyboard.look()
            shoot(KeyboardPanel(state: keyboard), "keyboard-trigger", size: keyboardSize, scheme: scheme)
        }
    }
}
#else
print("Shots draws on a Mac.")
#endif
