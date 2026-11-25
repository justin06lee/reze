import Foundation

// What the phone keeps of a Mac's library, so that it is there with no Mac in
// reach — and so that the keyboard, which is never allowed the network unless
// it is given Full Access, has something to read. The app writes it and the
// keyboard only ever reads it; they find each other through an App Group.

/// Which Mac the copy on the phone came from, and how it stood when fetched.
public struct Fetched: Codable, Hashable, Sendable {
    public var mac: Mac
    /// The Mac's revision at the time. The same one again means nothing to do.
    public var revision: String
    public var at: Date
    /// Whether that Mac shares its clipboard, as of then.
    public var clipboard: Bool

    public init(mac: Mac, revision: String = "", at: Date = Date(), clipboard: Bool = false) {
        self.mac = mac
        self.revision = revision
        self.at = at
        self.clipboard = clipboard
    }
}

public struct Shelf: Sendable {
    /// The App Group the app and its keyboard share. Also in `project.yml`
    /// and both entitlements files — the three have to agree.
    public static let group = "group.com.justin06lee.reze"

    public let root: URL

    public init(root: URL) {
        self.root = root
    }

    /// The shared container, or `nil` if the build was not given the group.
    public static func shared() -> Shelf? {
        FileManager.default
            .containerURL(forSecurityApplicationGroupIdentifier: group)
            .map { Shelf(root: $0.appendingPathComponent("Reze", isDirectory: true)) }
    }

    private var libraryFile: URL { root.appendingPathComponent("library.json") }
    private var fetchedFile: URL { root.appendingPathComponent("fetched.json") }
    private var assets: URL { root.appendingPathComponent("assets", isDirectory: true) }

    func read<T: Decodable>(_ type: T.Type, at url: URL) -> T? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        let decoder = JSONDecoder()
        decoder.dateDecodingStrategy = .millisecondsSince1970
        return try? decoder.decode(type, from: data)
    }

    func write<T: Encodable>(_ value: T, to url: URL) throws {
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let encoder = JSONEncoder()
        encoder.dateEncodingStrategy = .millisecondsSince1970
        // Atomically: the keyboard may be reading at this very moment, and
        // should find the old library or the new one, never half of either.
        try encoder.encode(value).write(to: url, options: .atomic)
    }

    public func library() -> Library? { read(Library.self, at: libraryFile) }
    public func fetched() -> Fetched? { read(Fetched.self, at: fetchedFile) }

    public func keep(_ library: Library) throws { try write(library, to: libraryFile) }
    public func keep(_ fetched: Fetched) throws { try write(fetched, to: fetchedFile) }

    /// Where an attachment's file is kept, or `nil` if its name is not one.
    ///
    /// The boundary the Mac holds its own library to: a bare file name or
    /// nothing, so a library cannot name its way out of the directory.
    public func url(for attachment: Attachment) -> URL? {
        let file = attachment.file
        let bare = attachment.fetchable
            && file != "." && file != ".."
            && !file.contains("/") && !file.contains("\\") && !file.contains("\0")
        return bare ? assets.appendingPathComponent(file) : nil
    }

    /// Whether the file is here. One that is, is whole: a download lands
    /// somewhere else and is only moved in once all of it has arrived.
    public func has(_ attachment: Attachment) -> Bool {
        url(for: attachment).map { FileManager.default.fileExists(atPath: $0.path) } ?? false
    }

    public func missing(from library: Library) -> [Attachment] {
        library.attachments.filter { url(for: $0) != nil && !has($0) }
    }

    /// Remove every file no macro names any more.
    public func sweep(keeping library: Library) {
        let named = Set(library.attachments.map(\.file))
        let here = (try? FileManager.default.contentsOfDirectory(atPath: assets.path)) ?? []
        for file in here where !named.contains(file) {
            try? FileManager.default.removeItem(at: assets.appendingPathComponent(file))
        }
    }

    /// How much room the attachments take.
    public func bytes() -> Int64 {
        let here = (try? FileManager.default.contentsOfDirectory(
            at: assets, includingPropertiesForKeys: [.fileSizeKey]
        )) ?? []
        return here.reduce(0) { $0 + Int64((try? $1.resourceValues(forKeys: [.fileSizeKey]).fileSize) ?? 0) }
    }

    /// Forget the Mac and everything fetched from it.
    public func empty() {
        try? FileManager.default.removeItem(at: root)
    }
}

/// What looking at the Mac again came to.
public struct Refreshed: Hashable, Sendable {
    /// Whether anything on the phone changed.
    public var changed: Bool
    public var macros: Int
    public var fetched: Int
    /// Attachments the Mac's library names that the Mac no longer has.
    public var gone: [String]
    /// Attachments that could not be fetched this time, and will be asked for
    /// again the next.
    public var failed: [String]
}

extension Shelf {
    /// Make the copy here the same as the Mac's: its macros, and the file of
    /// every attachment they name.
    ///
    /// One way only. Nothing done on the phone is ever sent to the Mac —
    /// there is nothing on the Mac that would take it.
    public func refresh(
        from client: Client,
        progress: @Sendable (_ done: Int, _ of: Int) -> Void = { _, _ in }
    ) async throws -> Refreshed {
        let hello = try await client.hello()
        let before = fetched()

        var library = self.library()
        var changed = false
        if library == nil || before?.mac != client.mac || before?.revision != hello.revision {
            let theirs = try await client.library()
            changed = theirs != library
            // The library first and its files after: a macro whose picture has
            // not arrived yet is still its text, which is most of most macros.
            try keep(theirs)
            library = theirs
        }
        guard let library else { throw Trouble.garbled }

        let wanted = missing(from: library)
        var fetched = 0
        var gone: [String] = []
        var failed: [String] = []
        for (i, attachment) in wanted.enumerated() {
            progress(i, wanted.count)
            guard let destination = url(for: attachment) else { continue }
            do {
                if try await client.asset(attachment.file, to: destination) {
                    fetched += 1
                } else {
                    gone.append(attachment.name)
                }
            } catch is CancellationError {
                throw CancellationError()
            } catch {
                failed.append(attachment.name)
            }
        }
        progress(wanted.count, wanted.count)
        sweep(keeping: library)

        try keep(Fetched(mac: client.mac, revision: hello.revision, at: Date(), clipboard: hello.clipboard))
        return Refreshed(
            changed: changed || fetched > 0,
            macros: library.macros.count,
            fetched: fetched,
            gone: gone,
            failed: failed
        )
    }
}
