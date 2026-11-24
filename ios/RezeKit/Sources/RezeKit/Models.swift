import Foundation

// The library as a Mac's Reze hands it over (`GET /library` is `macros.json`
// as it lies on disk). The desktop app owns that format and adds to it, so
// every field here is read if present and defaulted if not, and anything this
// file has never heard of is passed over rather than refused.

/// Which preview an attachment gets.
public enum AttachmentKind: String, Codable, Sendable {
    case image, video, audio, file

    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = AttachmentKind(rawValue: raw) ?? .file
    }
}

/// A file that goes along with a macro's text.
public struct Attachment: Codable, Identifiable, Hashable, Sendable {
    public var id: String
    /// A bare name inside the Mac's `~/.reze/assets`, and inside the cache here.
    public var file: String
    /// Set for a file the Mac references where it lies, which it does not serve.
    public var path: String?
    /// What it was called when it came in, for display only.
    public var name: String
    public var kind: AttachmentKind
    public var mime: String
    public var bytes: Int64

    public init(
        id: String = UUID().uuidString,
        file: String,
        path: String? = nil,
        name: String = "",
        kind: AttachmentKind = .file,
        mime: String = "application/octet-stream",
        bytes: Int64 = 0
    ) {
        self.id = id
        self.file = file
        self.path = path
        self.name = name
        self.kind = kind
        self.mime = mime
        self.bytes = bytes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        file = try c.decodeIfPresent(String.self, forKey: .file) ?? ""
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? file
        path = try c.decodeIfPresent(String.self, forKey: .path)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? file
        kind = try c.decodeIfPresent(AttachmentKind.self, forKey: .kind) ?? .file
        mime = try c.decodeIfPresent(String.self, forKey: .mime) ?? "application/octet-stream"
        bytes = try c.decodeIfPresent(Int64.self, forKey: .bytes) ?? 0
    }

    /// Whether a Mac will hand this file over: only the ones it keeps itself.
    public var fetchable: Bool { path == nil && !file.isEmpty }
}

public struct Macro: Codable, Identifiable, Hashable, Sendable {
    public var id: String
    /// What you type to find it, e.g. "full analysis".
    public var name: String
    public var description: String
    public var tags: [String]
    /// The expansion. May contain `{{vars}}` and `{{> includes}}`.
    public var body: String
    public var attachments: [Attachment]
    /// How often it was used on the Mac it came from. Never sent back.
    public var usageCount: Int
    /// When its contents last changed, in Unix milliseconds.
    public var updatedAt: Int64?

    public init(
        id: String = UUID().uuidString,
        name: String,
        description: String = "",
        tags: [String] = [],
        body: String = "",
        attachments: [Attachment] = [],
        usageCount: Int = 0,
        updatedAt: Int64? = nil
    ) {
        self.id = id
        self.name = name
        self.description = description
        self.tags = tags
        self.body = body
        self.attachments = attachments
        self.usageCount = usageCount
        self.updatedAt = updatedAt
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? ""
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? name
        description = try c.decodeIfPresent(String.self, forKey: .description) ?? ""
        tags = try c.decodeIfPresent([String].self, forKey: .tags) ?? []
        body = try c.decodeIfPresent(String.self, forKey: .body) ?? ""
        attachments = try c.decodeIfPresent([Attachment].self, forKey: .attachments) ?? []
        usageCount = try c.decodeIfPresent(Int.self, forKey: .usageCount) ?? 0
        updatedAt = try c.decodeIfPresent(Int64.self, forKey: .updatedAt)
    }
}

/// The macros, and nothing else of the file: a Mac's hotkeys, folders and
/// remembered deletions mean nothing on a phone, so they are not kept on one.
public struct Library: Codable, Hashable, Sendable {
    public var version: Int
    public var macros: [Macro]

    public init(version: Int = 1, macros: [Macro] = []) {
        self.version = version
        self.macros = macros
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(Int.self, forKey: .version) ?? 1
        macros = try c.decodeIfPresent([Macro].self, forKey: .macros) ?? []
    }

    /// Every attachment a Mac would hand over, each file once.
    public var attachments: [Attachment] {
        var seen = Set<String>()
        return macros.flatMap(\.attachments).filter { $0.fetchable && seen.insert($0.file).inserted }
    }
}

/// What a Mac says about itself when asked whether Reze is there (`GET /reze`).
public struct Hello: Codable, Hashable, Sendable {
    public var `protocol`: Int
    /// `macos` or `linux`.
    public var os: String
    public var macros: Int
    public var attachments: Int
    public var sync: Bool
    /// What its library holds. The same as last time means nothing to fetch.
    public var revision: String
    /// Whether it lets its other devices have what was last copied on it.
    public var clipboard: Bool

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        `protocol` = try c.decode(Int.self, forKey: .protocol)
        os = try c.decodeIfPresent(String.self, forKey: .os) ?? ""
        macros = try c.decodeIfPresent(Int.self, forKey: .macros) ?? 0
        attachments = try c.decodeIfPresent(Int.self, forKey: .attachments) ?? 0
        sync = try c.decodeIfPresent(Bool.self, forKey: .sync) ?? false
        revision = try c.decodeIfPresent(String.self, forKey: .revision) ?? ""
        clipboard = try c.decodeIfPresent(Bool.self, forKey: .clipboard) ?? false
    }
}

/// What was last copied on a Mac, as it hands it over (`GET /clip`).
public struct Clip: Codable, Hashable, Sendable {
    /// `text` or `image`.
    public var kind: String
    public var text: String
    /// For an image, the name to ask `/clip/` for.
    public var file: String
    public var mime: String
    public var bytes: Int64

    public init(kind: String, text: String = "", file: String = "", mime: String = "", bytes: Int64 = 0) {
        self.kind = kind
        self.text = text
        self.file = file
        self.mime = mime
        self.bytes = bytes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        kind = try c.decode(String.self, forKey: .kind)
        text = try c.decodeIfPresent(String.self, forKey: .text) ?? ""
        file = try c.decodeIfPresent(String.self, forKey: .file) ?? ""
        mime = try c.decodeIfPresent(String.self, forKey: .mime) ?? ""
        bytes = try c.decodeIfPresent(Int64.self, forKey: .bytes) ?? 0
    }
}

/// Human-sized file size. Decimal units, because that is what Finder shows.
public func formatBytes(_ bytes: Int64) -> String {
    if bytes < 1000 { return "\(bytes) B" }
    let units = ["kB", "MB", "GB", "TB"]
    var value = Double(bytes) / 1000
    var unit = 0
    while value >= 1000, unit < units.count - 1 {
        value /= 1000
        unit += 1
    }
    let shown = value < 10 ? String(format: "%.1f", value) : String(Int(value.rounded()))
    return "\(shown) \(units[unit])"
}
