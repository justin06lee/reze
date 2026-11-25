import Foundation
import RezeKit
import UniformTypeIdentifiers

#if canImport(UIKit)
import UIKit
#elseif canImport(AppKit)
import AppKit
#endif

/// The system clipboard, which is the one way anything leaves this app: a
/// phone lets nothing type into another app except a keyboard, so a macro is
/// copied here and pasted there.
public enum Clipboard {
    /// A file to put on the clipboard, and what kind of thing it is.
    public struct File {
        public var url: URL
        public var type: UTType

        public init(url: URL, type: UTType) {
            self.url = url
            self.type = type
        }

        /// An attachment's file, typed by what the Mac said it was and, failing
        /// that, by its extension — apps decide what they will take a paste of
        /// by this, so "some data" would be pasted nowhere.
        public init(_ attachment: Attachment, at url: URL) {
            self.url = url
            self.type = UTType(mimeType: attachment.mime)
                ?? UTType(filenameExtension: url.pathExtension)
                ?? .data
        }
    }

    /// The text on the clipboard, if there is any. On a phone the first read
    /// in a while is what brings up "Allow Paste".
    public static func text() -> String? {
        #if canImport(UIKit)
        return UIPasteboard.general.string
        #elseif canImport(AppKit)
        return NSPasteboard.general.string(forType: .string)
        #else
        return nil
        #endif
    }

    /// Replace the clipboard with `text` and `files`, each its own item.
    /// Returns how many of the files went on: one that cannot be read is left
    /// out rather than failing the rest.
    @discardableResult
    public static func set(text: String?, files: [File] = []) -> Int {
        let loaded = files.compactMap { file in
            (try? Data(contentsOf: file.url, options: .mappedIfSafe)).map { (file.type, $0) }
        }
        #if canImport(UIKit)
        var items: [[String: Any]] = []
        if let text, !text.isEmpty { items.append([UTType.utf8PlainText.identifier: text]) }
        for (type, data) in loaded { items.append([type.identifier: data]) }
        UIPasteboard.general.setItems(items, options: [:])
        #elseif canImport(AppKit)
        let board = NSPasteboard.general
        board.clearContents()
        var items: [NSPasteboardItem] = []
        if let text, !text.isEmpty {
            let item = NSPasteboardItem()
            item.setString(text, forType: .string)
            items.append(item)
        }
        for (type, data) in loaded {
            let item = NSPasteboardItem()
            item.setData(data, forType: NSPasteboard.PasteboardType(type.identifier))
            items.append(item)
        }
        board.writeObjects(items)
        #endif
        return loaded.count
    }

    /// Replace the clipboard with one image, as it arrived.
    public static func set(image data: Data, mime: String) {
        let type = UTType(mimeType: mime) ?? .png
        #if canImport(UIKit)
        UIPasteboard.general.setItems([[type.identifier: data]], options: [:])
        #elseif canImport(AppKit)
        let board = NSPasteboard.general
        board.clearContents()
        board.setData(data, forType: NSPasteboard.PasteboardType(type.identifier))
        #endif
    }
}

/// The clipboard as the clip list needs it: whether it has changed, and what
/// is on it. A value rather than calls to the system so that tests, and the
/// tool that draws the screens, never touch the real one.
public struct Pasteboard {
    /// A number that moves each time anything is copied. Looking at it costs
    /// nothing and asks nobody.
    public var count: () -> Int
    /// What is on it. On a phone this is the read that can bring up "Allow
    /// Paste". `decoding` says whether an image offered in no ready-made
    /// format may be redrawn as one, which takes memory a keyboard lacks.
    public var read: (_ decoding: Bool) -> Copied?

    public init(count: @escaping () -> Int, read: @escaping (_ decoding: Bool) -> Copied?) {
        self.count = count
        self.read = read
    }

    public static let system = Pasteboard(count: { Clipboard.changeCount }, read: { Clipboard.current(decoding: $0) })

    /// What is on it, if it has changed since `shelf` last saw it.
    func fresh(for shelf: Shelf, decoding: Bool) -> Copied? {
        let now = count()
        guard shelf.pasteboardSeen() != now else { return nil }
        shelf.sawPasteboard(now)
        return read(decoding)
    }
}

extension Clipboard {
    public static var changeCount: Int {
        #if canImport(UIKit)
        return UIPasteboard.general.changeCount
        #elseif canImport(AppKit)
        return NSPasteboard.general.changeCount
        #else
        return 0
        #endif
    }

    /// What is on the clipboard, as far as Reze carries it: an image as the
    /// PNG or JPEG it was copied as, or a text.
    public static func current(decoding: Bool) -> Copied? {
        #if canImport(UIKit)
        let board = UIPasteboard.general
        if board.hasImages {
            for (type, mime) in [(UTType.png, "image/png"), (UTType.jpeg, "image/jpeg")] {
                if let data = board.data(forPasteboardType: type.identifier), !data.isEmpty {
                    return Int64(data.count) <= Shelf.largestClip ? .image(data, mime: mime) : nil
                }
            }
            if decoding, let data = board.image?.jpegData(compressionQuality: 0.9) {
                return .image(data, mime: "image/jpeg")
            }
        }
        if board.hasStrings, let text = board.string, !text.isEmpty { return .text(text) }
        return nil
        #elseif canImport(AppKit)
        let board = NSPasteboard.general
        if let text = board.string(forType: .string), !text.isEmpty { return .text(text) }
        if let data = board.data(forType: .png) { return .image(data, mime: "image/png") }
        return nil
        #else
        return nil
        #endif
    }

    /// Put a remembered copy back on the clipboard.
    @discardableResult
    public static func set(_ clip: ClipEntry, in shelf: Shelf) -> Bool {
        if clip.isImage {
            guard let url = shelf.url(for: clip), let data = try? Data(contentsOf: url, options: .mappedIfSafe) else { return false }
            set(image: data, mime: clip.mime)
        } else {
            set(text: clip.text)
        }
        return true
    }

    /// What this device is called, for the Mac's history to say where a copy
    /// came from. iOS hands apps "iPhone" or "iPad" and nothing more personal.
    public static var deviceName: String {
        #if canImport(UIKit)
        return UIDevice.current.userInterfaceIdiom == .pad ? "iPad" : "iPhone"
        #else
        return "Mac"
        #endif
    }
}
