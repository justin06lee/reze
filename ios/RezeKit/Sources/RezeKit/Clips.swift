import CryptoKit
import Foundation

// The clipboard on the phone: what was copied here, and what was last copied
// on the Mac, in one list the app and the keyboard both read.
//
// A phone will not let anything watch its clipboard. So the list is brought
// up to date whenever Reze is in front of the person — the app on screen, the
// keyboard up — and each time that happens the two sides are evened out
// (`exchange`): the Mac's newest copy is taken, and the phone's newest is
// handed to the Mac, which puts it on its own clipboard and tells the
// machines it shares with.

/// Something on the phone's clipboard, as far as Reze takes it.
public enum Copied: Hashable, Sendable {
    case text(String)
    case image(Data, mime: String)

    var digest: String {
        let bytes: Data
        switch self {
        case .text(let text): bytes = Data(text.utf8)
        case .image(let data, _): bytes = data
        }
        return SHA256.hash(data: bytes).map { String(format: "%02x", $0) }.joined()
    }
}

/// One remembered copy.
public struct ClipEntry: Codable, Identifiable, Hashable, Sendable {
    public var id: String
    /// `text` or `image`.
    public var kind: String
    public var text: String
    /// For an image, a bare name inside the phone's `clips` directory.
    public var file: String
    public var mime: String
    public var bytes: Int64
    public var at: Date
    /// The Mac it was copied on, or empty for this phone.
    public var from: String
    /// Whether the Mac has it: it came from there, or was handed to it.
    public var shared: Bool
    /// What it holds, hashed, so the same thing is not kept twice.
    public var digest: String
    /// For an image from the Mac, the name the Mac knows it by.
    public var origin: String

    public var isImage: Bool { kind == "image" }
    /// Whether it was copied on this phone.
    public var mine: Bool { from.isEmpty }
}

/// What evening the phone and the Mac out came to.
public struct Exchanged: Hashable, Sendable {
    /// What was newly taken from the phone's clipboard.
    public var took: ClipEntry?
    /// Whether the phone's newest copy was handed to the Mac this time.
    public var sent = false
    /// What newly arrived from the Mac.
    public var arrived: ClipEntry?
    /// Whether the Mac shares its clipboard. `nil` if it could not be asked.
    public var sharing: Bool?
    /// Why the Mac could not be asked, or would not take what it was handed.
    public var trouble: String?
}

extension Shelf {
    /// How many copies are kept.
    public static let clipLimit = 40
    /// The largest image worth carrying either way — the Mac's own limit.
    public static let largestClip: Int64 = 32 * 1024 * 1024

    private var clipsFile: URL { root.appendingPathComponent("clips.json") }
    private var clipsDir: URL { root.appendingPathComponent("clips", isDirectory: true) }
    private var seenFile: URL { root.appendingPathComponent("pasteboard.json") }

    public func clips() -> [ClipEntry] { read([ClipEntry].self, at: clipsFile) ?? [] }

    /// Where a copied image is kept, or `nil` if the entry is not one.
    public func url(for clip: ClipEntry) -> URL? {
        let file = clip.file
        let bare = clip.isImage && !file.isEmpty && file != "." && file != ".."
            && !file.contains("/") && !file.contains("\\") && !file.contains("\0")
        return bare ? clipsDir.appendingPathComponent(file) : nil
    }

    /// The clipboard's change count as of the last time it was read. The
    /// clipboard is only read again once this has moved: reading is what
    /// makes a phone ask "Allow Paste", and what tells a fresh copy from one
    /// that has been sitting there.
    public func pasteboardSeen() -> Int? { read(Int.self, at: seenFile) }
    public func sawPasteboard(_ count: Int) { try? write(count, to: seenFile) }

    /// Remember a copy. Returns the entry, and whether it is new to the list.
    ///
    /// The same thing copied again on the phone is one entry, moved to the
    /// top. The same thing arriving from the Mac is left where it is: the Mac
    /// says what its newest copy is, not when it was made.
    @discardableResult
    public func record(
        _ copied: Copied,
        from: String,
        shared: Bool,
        origin: String = "",
        now: Date = Date()
    ) throws -> (entry: ClipEntry, new: Bool)? {
        if case .text(let text) = copied, text.allSatisfy(\.isWhitespace) { return nil }
        var all = clips()
        let digest = copied.digest

        if let at = all.firstIndex(where: { $0.digest == digest }) {
            var known = all.remove(at: at)
            if !origin.isEmpty { known.origin = origin }
            if shared { known.shared = true }
            if from.isEmpty { known.at = now }
            all.insert(known, at: from.isEmpty ? 0 : at)
            try write(all, to: clipsFile)
            return (known, false)
        }

        var entry = ClipEntry(
            id: UUID().uuidString, kind: "text", text: "", file: "", mime: "text/plain", bytes: 0,
            at: now, from: from, shared: shared, digest: digest, origin: origin
        )
        switch copied {
        case .text(let text):
            entry.text = text
            entry.bytes = Int64(text.utf8.count)
        case .image(let data, let mime):
            let ext = ["image/png": "png", "image/jpeg": "jpg", "image/tiff": "tiff", "image/gif": "gif"][mime] ?? "bin"
            entry.kind = "image"
            entry.mime = mime
            entry.bytes = Int64(data.count)
            entry.file = "\(entry.id).\(ext)"
            try FileManager.default.createDirectory(at: clipsDir, withIntermediateDirectories: true)
            try data.write(to: clipsDir.appendingPathComponent(entry.file), options: .atomic)
        }
        all.insert(entry, at: 0)
        for dropped in all.dropFirst(Shelf.clipLimit) {
            if let url = url(for: dropped) { try? FileManager.default.removeItem(at: url) }
        }
        try write(Array(all.prefix(Shelf.clipLimit)), to: clipsFile)
        return (entry, true)
    }

    private func change(_ id: String, _ change: (inout ClipEntry) -> Void) {
        var all = clips()
        guard let at = all.firstIndex(where: { $0.id == id }) else { return }
        change(&all[at])
        try? write(all, to: clipsFile)
    }

    /// Forget every copy kept on the phone.
    public func clearClips() {
        try? FileManager.default.removeItem(at: clipsFile)
        try? FileManager.default.removeItem(at: clipsDir)
    }

    /// Even the phone and the Mac out: take the Mac's newest copy, remember
    /// the phone's (`copied`, if its clipboard has changed), and hand the
    /// phone's newest to the Mac if the Mac does not have it yet.
    ///
    /// `name` is what the Mac's history will say it came from.
    public func exchange(with client: Client, copied: Copied?, name: String) async -> Exchanged {
        var out = Exchanged()
        do {
            if let clip = try await client.clip() {
                out.sharing = true
                out.arrived = try await take(clip, from: client)
            } else {
                out.sharing = false
            }
        } catch is CancellationError {
            return out
        } catch {
            out.trouble = error.localizedDescription
        }

        // What was copied here is remembered whether or not the Mac is there.
        if let copied, let recorded = try? record(copied, from: "", shared: false), recorded.new {
            out.took = recorded.entry
        }

        // And handed over, if it is the newest thing here and the Mac lacks
        // it — which also covers a copy made while the Mac was out of reach,
        // unless the Mac has copied something of its own since: then which
        // was later cannot be known, and the Mac's clipboard is left alone.
        guard out.sharing == true, let newest = clips().first, newest.mine, !newest.shared else { return out }
        do {
            let taken: Bool
            if newest.isImage {
                guard let url = url(for: newest), let data = try? Data(contentsOf: url) else { return out }
                taken = try await client.send(image: data, mime: newest.mime, from: name)
            } else {
                taken = try await client.send(text: newest.text, from: name)
            }
            if taken {
                change(newest.id) { $0.shared = true }
                out.sent = true
            }
        } catch Trouble.status(400) {
            out.trouble = "Reze on \(client.mac.label) is too old to take what the phone copies. Update it there."
        } catch is CancellationError {
        } catch {
            out.trouble = error.localizedDescription
        }
        return out
    }

    /// Remember the Mac's newest copy, if it is not already here.
    private func take(_ clip: Clip, from client: Client) async throws -> ClipEntry? {
        let label = client.mac.label
        switch clip.kind {
        case "text":
            let recorded = try record(.text(clip.text), from: label, shared: true)
            return recorded?.new == true ? recorded?.entry : nil
        case "image":
            // Known by the Mac's own name for it: not fetched a second time.
            if clips().contains(where: { !$0.origin.isEmpty && $0.origin == clip.file }) { return nil }
            guard clip.bytes <= Shelf.largestClip, let data = try await client.image(of: clip) else { return nil }
            let recorded = try record(.image(data, mime: clip.mime), from: label, shared: true, origin: clip.file)
            return recorded?.new == true ? recorded?.entry : nil
        default:
            return nil
        }
    }
}
