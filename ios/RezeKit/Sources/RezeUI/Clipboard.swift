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
