import Foundation
import Testing

@testable import RezeKit

// A Mac, as far as the client can tell: answers by path, the way the
// listener in `src-tauri/src/tailnet.rs` does. Each test has one of its own,
// told apart by host name, so they can run side by side.

final class PretendMac: URLProtocol, @unchecked Sendable {
    struct Answers: Sendable {
        var paths: [String: Data] = [:]
        /// A status every request gets instead, as a Mac refusing a stranger does.
        var refuse: Int?
        var asked: [String] = []
    }

    private static let lock = NSLock()
    nonisolated(unsafe) private static var macs: [String: Answers] = [:]

    static func set(_ host: String, _ change: (inout Answers) -> Void) {
        lock.lock()
        defer { lock.unlock() }
        var answers = macs[host] ?? Answers()
        change(&answers)
        macs[host] = answers
    }

    static func asked(_ host: String) -> [String] {
        lock.lock()
        defer { lock.unlock() }
        return macs[host]?.asked ?? []
    }

    static func client(_ host: String) -> Client {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [PretendMac.self]
        return Client(Mac(host: host), session: URLSession(configuration: config))
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}

    override func startLoading() {
        guard let url = request.url, let host = url.host else { return }
        // The path as it went over the wire, escapes and all.
        let path = URLComponents(url: url, resolvingAgainstBaseURL: false)?.percentEncodedPath ?? url.path

        PretendMac.lock.lock()
        let known = PretendMac.macs[host]
        PretendMac.macs[host]?.asked.append(path)
        PretendMac.lock.unlock()

        guard let known else {
            client?.urlProtocol(self, didFailWithError: URLError(.cannotConnectToHost))
            return
        }
        let body = known.paths[path]
        let status = known.refuse ?? (body == nil ? 404 : 200)
        let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: status == 200 ? body! : Data("no".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
}

private func hello(revision: String, clipboard: Bool = false) -> Data {
    Data(#"{"protocol":1,"os":"macos","macros":2,"attachments":2,"sync":true,"revision":"\#(revision)","clipboard":\#(clipboard)}"#.utf8)
}

private let twoMacros = Data("""
{
  "version": 1,
  "settings": { "hotkey": "Alt+Space" },
  "macros": [
    { "id": "a", "name": "greeting", "body": "Hello {{name|there}}", "attachments": [], "usageCount": 4 },
    { "id": "b", "name": "meme", "body": "", "usageCount": 1, "attachments": [
      { "id": "1", "file": "cat pic.png", "name": "cat.png", "kind": "image", "mime": "image/png", "bytes": 3 },
      { "id": "2", "file": "lost.mp4", "name": "lost.mp4", "kind": "video", "mime": "video/mp4", "bytes": 9 },
      { "id": "3", "file": "../../escape", "name": "sneaky", "kind": "file", "mime": "x/y", "bytes": 1 },
      { "id": "4", "file": "", "path": "/Users/x/y.png", "name": "y.png", "kind": "image", "mime": "image/png", "bytes": 1 }
    ] }
  ]
}
""".utf8)

private func scratch() throws -> Shelf {
    let root = FileManager.default.temporaryDirectory
        .appendingPathComponent("rezekit-\(UUID().uuidString)", isDirectory: true)
    return Shelf(root: root.appendingPathComponent("Reze", isDirectory: true))
}

@Test func aLibraryAndItsFilesAreFetchedOnceAndKept() async throws {
    let host = "fetch-once"
    PretendMac.set(host) {
        $0.paths = [
            "/reze": hello(revision: "r1", clipboard: true),
            "/library": twoMacros,
            "/assets/cat%20pic.png": Data([1, 2, 3]),
        ]
    }
    let shelf = try scratch()
    defer { shelf.empty() }
    let client = PretendMac.client(host)

    let first = try await shelf.refresh(from: client)
    #expect(first.changed)
    #expect(first.macros == 2)
    #expect(first.fetched == 1)
    // Named by the library and gone from the Mac: said, and not fatal.
    #expect(first.gone == ["lost.mp4"])
    #expect(first.failed.isEmpty)

    let library = try #require(shelf.library())
    #expect(library.macros.map(\.name) == ["greeting", "meme"])
    let cat = library.macros[1].attachments[0]
    #expect(shelf.has(cat))
    #expect(try Data(contentsOf: #require(shelf.url(for: cat))) == Data([1, 2, 3]))
    // A name that is a path is never asked for and never has anywhere to go.
    #expect(shelf.url(for: library.macros[1].attachments[2]) == nil)
    #expect(!PretendMac.asked(host).contains { $0.contains("escape") })

    let fetched = try #require(shelf.fetched())
    #expect(fetched.mac == client.mac)
    #expect(fetched.revision == "r1")
    #expect(fetched.clipboard)

    // The same revision again: the library is not asked for a second time.
    let again = try await shelf.refresh(from: client)
    #expect(!again.changed)
    #expect(PretendMac.asked(host).filter { $0 == "/library" }.count == 1)
    #expect(PretendMac.asked(host).filter { $0 == "/assets/cat%20pic.png" }.count == 1)
}

@Test func whatTheMacDeletedIsSweptFromThePhone() async throws {
    let host = "sweeps"
    PretendMac.set(host) {
        $0.paths = [
            "/reze": hello(revision: "r1"),
            "/library": twoMacros,
            "/assets/cat%20pic.png": Data([1, 2, 3]),
        ]
    }
    let shelf = try scratch()
    defer { shelf.empty() }
    let client = PretendMac.client(host)
    _ = try await shelf.refresh(from: client)
    let cat = try #require(shelf.library()).macros[1].attachments[0]
    #expect(shelf.has(cat))
    #expect(shelf.bytes() == 3)

    PretendMac.set(host) {
        $0.paths["/reze"] = hello(revision: "r2")
        $0.paths["/library"] = Data(#"{"version":1,"macros":[{"id":"a","name":"greeting","body":"Hi"}]}"#.utf8)
    }
    let after = try await shelf.refresh(from: client)
    #expect(after.changed)
    #expect(after.macros == 1)
    #expect(!shelf.has(cat))
    #expect(shelf.bytes() == 0)
}

@Test func aMacThatIsNotThereLeavesWhatWasFetched() async throws {
    let host = "goes-away"
    PretendMac.set(host) { $0.paths = ["/reze": hello(revision: "r1"), "/library": twoMacros] }
    let shelf = try scratch()
    defer { shelf.empty() }
    _ = try await shelf.refresh(from: PretendMac.client(host))

    // Nothing was ever told about this host, so nothing answers at it.
    await #expect(throws: Trouble.unreachable("nobody-home")) {
        try await shelf.refresh(from: PretendMac.client("nobody-home"))
    }
    #expect(shelf.library()?.macros.count == 2)
    #expect(shelf.fetched()?.mac.host == host)
}

@Test func aRefusalAndAStrangerAreToldApart() async throws {
    PretendMac.set("refuses") { $0.refuse = 403 }
    await #expect(throws: Trouble.refused) { try await PretendMac.client("refuses").hello() }

    PretendMac.set("web-server") { $0.paths = ["/reze": Data("<html>hello</html>".utf8)] }
    await #expect(throws: Trouble.notReze) { try await PretendMac.client("web-server").hello() }

    PretendMac.set("empty") { _ in }
    await #expect(throws: Trouble.notReze) { try await PretendMac.client("empty").hello() }

    PretendMac.set("future") { $0.paths = ["/reze": Data(#"{"protocol":2}"#.utf8)] }
    await #expect(throws: Trouble.garbled) { try await PretendMac.client("future").hello() }

    PretendMac.set("broken") { $0.refuse = 500 }
    await #expect(throws: Trouble.status(500)) { try await PretendMac.client("broken").hello() }
}

@Test func theClipboardIsThereOrItIsNot() async throws {
    PretendMac.set("shares") {
        $0.paths = [
            "/clip": Data(#"{"kind":"image","file":"shot 1.png","mime":"image/png","bytes":4}"#.utf8),
            "/clip/shot%201.png": Data([9, 9, 9, 9]),
        ]
    }
    let client = PretendMac.client("shares")
    let clip = try #require(try await client.clip())
    #expect(clip.kind == "image" && clip.text.isEmpty)
    #expect(try await client.image(of: clip) == Data([9, 9, 9, 9]))

    // Sharing switched off there is a 404, which is an answer, not a failure.
    PretendMac.set("keeps-to-itself") { _ in }
    #expect(try await PretendMac.client("keeps-to-itself").clip() == nil)
}

/// Against a real Mac, when one is named: `REZE_LIVE=100.71.90.33 swift test`.
/// Reads only; nothing on the Mac changes.
@Test(.enabled(if: ProcessInfo.processInfo.environment["REZE_LIVE"] != nil))
func aRealMacAnswers() async throws {
    let mac = try #require(Mac(typed: ProcessInfo.processInfo.environment["REZE_LIVE"] ?? ""))
    let client = Client(mac)
    let hello = try await client.hello()
    #expect(hello.protocol == 1)

    let shelf = try scratch()
    defer { shelf.empty() }
    let refreshed = try await shelf.refresh(from: client)
    #expect(refreshed.macros == hello.macros)
    #expect(refreshed.failed.isEmpty)
    let library = try #require(shelf.library())
    #expect(shelf.missing(from: library).count == refreshed.gone.count)
    print("live: \(refreshed.macros) macros, \(refreshed.fetched) files (\(formatBytes(shelf.bytes()))), gone: \(refreshed.gone)")
}
