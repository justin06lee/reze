import Foundation

// Asking a Mac's Reze for things. It listens on its Tailscale address only,
// answers `GET` and nothing else, and only to devices signed in to the tailnet
// as the same person — see "What is listening" in the README. Plain HTTP is
// what it speaks: Tailscale is what encrypts it and what says who is asking.

/// Which Mac, as it was typed in: a tailnet name or a Tailscale address.
public struct Mac: Codable, Hashable, Sendable {
    public static let port = 7393

    public var host: String
    public var port: Int

    public init(host: String, port: Int = Mac.port) {
        self.host = host
        self.port = port
    }

    /// Read what someone typed or pasted: `mac-mini`, `100.71.90.33`,
    /// `mac-mini.tail1234.ts.net`, with or without `http://`, a port or a path.
    public init?(typed: String) {
        var rest = typed.trimmingCharacters(in: .whitespacesAndNewlines).lowercased()
        if let scheme = rest.range(of: "://") { rest = String(rest[scheme.upperBound...]) }
        if let slash = rest.firstIndex(of: "/") { rest = String(rest[..<slash]) }
        var port = Mac.port
        if let colon = rest.lastIndex(of: ":") {
            guard let given = Int(rest[rest.index(after: colon)...]), (1...65535).contains(given) else {
                return nil
            }
            port = given
            rest = String(rest[..<colon])
        }
        // A name's last dot is harmless and MagicDNS names are often pasted
        // with it.
        if rest.hasSuffix(".") { rest.removeLast() }
        let allowed = Set("abcdefghijklmnopqrstuvwxyz0123456789-.")
        guard !rest.isEmpty, rest.allSatisfy(allowed.contains), !rest.hasPrefix("."), !rest.contains("..") else {
            return nil
        }
        self.init(host: rest, port: port)
    }

    /// What to call it: the first label of a name, or the address as it is.
    public var label: String {
        let isAddress = host.allSatisfy { $0.isNumber || $0 == "." }
        return isAddress ? host : String(host.split(separator: ".").first ?? Substring(host))
    }
}

/// Why asking came to nothing, worded for whoever is holding the phone.
public enum Trouble: Error, Equatable, LocalizedError, Sendable {
    /// Nothing answered at all.
    case unreachable(String)
    /// Something answered that the Mac will not serve to this device.
    case refused
    /// Something answered, and it was not Reze.
    case notReze
    /// Reze answered with something this app does not understand.
    case garbled
    case status(Int)

    public var errorDescription: String? {
        switch self {
        case .unreachable(let host):
            return "Nothing answered at \(host). Check that Tailscale is connected on this phone, "
                + "and that Reze is running on the Mac with \u{201C}Let my other devices import from this one\u{201D} on."
        case .refused:
            return "That Mac only answers devices signed in to Tailscale as the same person it is."
        case .notReze:
            return "Something answered there, but it was not Reze."
        case .garbled:
            return "Reze answered with something this app does not understand. One of the two may need updating."
        case .status(let code):
            return "The Mac answered with an error (\(code))."
        }
    }
}

public struct Client: Sendable {
    public let mac: Mac
    private let session: URLSession

    /// The protocol this app speaks. A Mac that says another number is not
    /// asked for anything more.
    public static let `protocol` = 1

    public init(_ mac: Mac, session: URLSession? = nil) {
        self.mac = mac
        self.session = session ?? {
            let config = URLSessionConfiguration.ephemeral
            // A Mac that is asleep takes this long to not answer, so it is
            // short; a file is given as long as it keeps arriving.
            config.timeoutIntervalForRequest = 8
            config.timeoutIntervalForResource = 600
            config.waitsForConnectivity = false
            config.requestCachePolicy = .reloadIgnoringLocalCacheData
            return URLSession(configuration: config)
        }()
    }

    /// A file name as one segment of a path: everything but the unreserved
    /// characters escaped, which is how the Mac escapes them itself.
    static func encode(_ name: String) -> String {
        var out = ""
        for byte in name.utf8 {
            switch byte {
            case UInt8(ascii: "A")...UInt8(ascii: "Z"), UInt8(ascii: "a")...UInt8(ascii: "z"),
                 UInt8(ascii: "0")...UInt8(ascii: "9"),
                 UInt8(ascii: "-"), UInt8(ascii: "."), UInt8(ascii: "_"), UInt8(ascii: "~"):
                out.unicodeScalars.append(Unicode.Scalar(byte))
            default:
                out += String(format: "%%%02X", byte)
            }
        }
        return out
    }

    private func url(_ path: String) throws -> URL {
        guard let url = URL(string: "http://\(mac.host):\(mac.port)\(path)") else {
            throw Trouble.unreachable(mac.host)
        }
        return url
    }

    private func check(_ response: URLResponse) throws -> Int {
        guard let http = response as? HTTPURLResponse else { throw Trouble.notReze }
        switch http.statusCode {
        case 200, 404: return http.statusCode
        case 403: throw Trouble.refused
        default: throw Trouble.status(http.statusCode)
        }
    }

    /// The body at `path`, or `nil` if the Mac has nothing by that name.
    private func get(_ path: String) async throws -> Data? {
        do {
            let (data, response) = try await session.data(from: try url(path))
            return try check(response) == 200 ? data : nil
        } catch let trouble as Trouble {
            throw trouble
        } catch is CancellationError {
            throw CancellationError()
        } catch let error as URLError where error.code == .cancelled {
            throw CancellationError()
        } catch {
            throw Trouble.unreachable(mac.host)
        }
    }

    private func decode<T: Decodable>(_ type: T.Type, from data: Data) throws -> T {
        do {
            return try JSONDecoder().decode(type, from: data)
        } catch {
            throw Trouble.garbled
        }
    }

    /// Whether Reze is there, and what it has.
    public func hello() async throws -> Hello {
        guard let data = try await get("/reze") else { throw Trouble.notReze }
        guard let hello = try? JSONDecoder().decode(Hello.self, from: data) else { throw Trouble.notReze }
        guard hello.protocol == Client.protocol else { throw Trouble.garbled }
        return hello
    }

    public func library() async throws -> Library {
        guard let data = try await get("/library") else { throw Trouble.notReze }
        return try decode(Library.self, from: data)
    }

    /// Fetch an attachment's file to `destination`. `false` if the Mac's
    /// library names it but the Mac no longer has it.
    @discardableResult
    public func asset(_ file: String, to destination: URL) async throws -> Bool {
        try await download("/assets/\(Client.encode(file))", to: destination)
    }

    /// What was last copied on the Mac, or `nil` if it shares no clipboard —
    /// because that is switched off there, or nothing has been copied yet.
    public func clip() async throws -> Clip? {
        guard let data = try await get("/clip") else { return nil }
        return try decode(Clip.self, from: data)
    }

    /// The image `clip` names. `nil` if something else has been copied since.
    public func image(of clip: Clip) async throws -> Data? {
        try await get("/clip/\(Client.encode(clip.file))")
    }

    /// Hand the Mac a text copied on the phone, for its clipboard. `false`
    /// if the Mac shares no clipboard.
    public func send(text: String, from name: String) async throws -> Bool {
        try await post("/clip", Data(text.utf8), type: "text/plain; charset=utf-8", from: name)
    }

    /// The same, for an image: PNG, JPEG or TIFF, as it is.
    public func send(image: Data, mime: String, from name: String) async throws -> Bool {
        try await post("/clip", image, type: mime, from: name)
    }

    /// The one request that brings the Mac something. A Mac that predates it
    /// answers 400, which comes out as `Trouble.status(400)`.
    private func post(_ path: String, _ body: Data, type: String, from name: String) async throws -> Bool {
        var request = URLRequest(url: try url(path))
        request.httpMethod = "POST"
        request.setValue(type, forHTTPHeaderField: "Content-Type")
        // Header values are ASCII; a name that is not is left for the Mac to
        // make up.
        let plain = String(name.unicodeScalars.filter { $0.isASCII && $0.value >= 0x20 && $0.value != 0x7f }.prefix(40))
        if !plain.isEmpty { request.setValue(plain, forHTTPHeaderField: "X-Reze-From") }
        do {
            let (_, response) = try await session.upload(for: request, from: body)
            return try check(response) == 200
        } catch let trouble as Trouble {
            throw trouble
        } catch is CancellationError {
            throw CancellationError()
        } catch let error as URLError where error.code == .cancelled {
            throw CancellationError()
        } catch {
            throw Trouble.unreachable(mac.host)
        }
    }

    private func download(_ path: String, to destination: URL) async throws -> Bool {
        do {
            let (landed, response) = try await session.download(from: try url(path))
            defer { try? FileManager.default.removeItem(at: landed) }
            guard try check(response) == 200 else { return false }
            let manager = FileManager.default
            try manager.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
            if manager.fileExists(atPath: destination.path) {
                _ = try manager.replaceItemAt(destination, withItemAt: landed)
            } else {
                try manager.moveItem(at: landed, to: destination)
            }
            return true
        } catch let trouble as Trouble {
            throw trouble
        } catch is CancellationError {
            throw CancellationError()
        } catch let error as URLError where error.code == .cancelled {
            throw CancellationError()
        } catch is URLError {
            throw Trouble.unreachable(mac.host)
        }
    }
}
