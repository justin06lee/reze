import Foundation
import Testing

import RezeKit
@testable import RezeUI

// The clipboard in the keyboard and the app, with a clipboard made of a
// variable and a Mac made of a stub: the real ones are somebody's.

/// A Mac that shares its clipboard: says what it last copied, takes what it is handed.
final class SharingMac: URLProtocol, @unchecked Sendable {
    private static let lock = NSLock()
    nonisolated(unsafe) private static var latest = "from the Mac"
    nonisolated(unsafe) private static var taken: [String] = []

    static func reset(latest text: String) {
        lock.lock(); defer { lock.unlock() }
        latest = text
        taken = []
    }

    static var handed: [String] {
        lock.lock(); defer { lock.unlock() }
        return taken
    }

    static func client(_ mac: Mac) -> Client {
        let config = URLSessionConfiguration.ephemeral
        config.protocolClasses = [SharingMac.self]
        return Client(mac, session: URLSession(configuration: config))
    }

    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func stopLoading() {}

    override func startLoading() {
        guard let url = request.url else { return }
        var body = Data("{}".utf8)
        var status = 200
        if request.httpMethod == "POST" {
            var sent = request.httpBody ?? Data()
            if let stream = request.httpBodyStream {
                stream.open()
                var buffer = [UInt8](repeating: 0, count: 4096)
                while stream.hasBytesAvailable {
                    let read = stream.read(&buffer, maxLength: buffer.count)
                    if read <= 0 { break }
                    sent.append(buffer, count: read)
                }
                stream.close()
            }
            SharingMac.lock.lock()
            SharingMac.taken.append(String(decoding: sent, as: UTF8.self))
            SharingMac.latest = String(decoding: sent, as: UTF8.self)
            SharingMac.lock.unlock()
        } else if url.path == "/clip" {
            SharingMac.lock.lock()
            let text = SharingMac.latest
            SharingMac.lock.unlock()
            body = try! JSONSerialization.data(withJSONObject: ["kind": "text", "text": text])
        } else {
            status = 404
        }
        let response = HTTPURLResponse(url: url, statusCode: status, httpVersion: "HTTP/1.1", headerFields: nil)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: body)
        client?.urlProtocolDidFinishLoading(self)
    }
}

@MainActor
private final class FakeBoard {
    var count = 1
    var text: String? = "copied on the phone"
    var reads = 0
    var board: Pasteboard {
        Pasteboard(count: { self.count }, read: { _ in
            self.reads += 1
            return self.text.map { .text($0) }
        })
    }
}

@MainActor
private func fetchedShelf() throws -> Shelf {
    let shelf = Shelf(root: FileManager.default.temporaryDirectory.appendingPathComponent("rezeui-\(UUID().uuidString)"))
    try shelf.keep(Library(macros: [Macro(name: "hello", body: "Hello")]))
    try shelf.keep(Fetched(mac: Mac(host: "mini"), clipboard: false))
    return shelf
}

// One test, in order: the stub Mac is shared, and tests run side by side.
@MainActor
@Test func theKeyboardAndTheAppShareTheClipboardWithTheMac() async throws {
    SharingMac.reset(latest: "from the Mac")
    let shelf = try fetchedShelf()
    defer { shelf.empty() }
    let board = FakeBoard()
    var typed = ""
    var fullAccess = false
    let proxy = KeyboardState.Proxy(
        insert: { typed += $0 }, deleteBackward: { _ = typed.popLast() }, nextKeyboard: {},
        context: { typed }, fullAccess: { fullAccess }, needsGlobe: { false }
    )
    let keyboard = KeyboardState(shelf: shelf, proxy: proxy, pasteboard: board.board, connect: SharingMac.client)
    keyboard.reload()

    // Without Full Access the clipboard is not so much as looked at.
    await keyboard.tick()
    #expect(board.reads == 0 && keyboard.clips.isEmpty)
    #expect(keyboard.clipNote?.contains("Full Access") == true)

    // With it: the phone's copy is taken and handed over, the Mac's arrives.
    fullAccess = true
    keyboard.look()
    await keyboard.tick()
    #expect(board.reads == 1)
    #expect(keyboard.clips.map(\.text) == ["copied on the phone", "from the Mac"])
    #expect(SharingMac.handed == ["copied on the phone"])
    #expect(keyboard.clipNote == nil)
    // And the phone now knows the Mac shares, for the app to say so.
    #expect(shelf.fetched()?.clipboard == true)

    // Nothing copied since: the clipboard is not read again, nothing is sent again.
    await keyboard.tick(now: Date().addingTimeInterval(60))
    #expect(board.reads == 1 && SharingMac.handed.count == 1)

    // A new copy is noticed at the next tick, whenever that is.
    board.count = 2
    board.text = "a second thing"
    await keyboard.tick()
    #expect(keyboard.clips.first?.text == "a second thing")
    #expect(SharingMac.handed.last == "a second thing")

    // Tapping a text types it.
    keyboard.use(try #require(keyboard.clips.last))
    #expect(typed == "from the Mac")

    // The app sees the same list, and takes what the Mac copies meanwhile.
    SharingMac.reset(latest: "later, on the Mac")
    let phone = Phone(shelf: shelf, pasteboard: board.board, connect: SharingMac.client)
    #expect(phone.clips.count == 3 && phone.sharing)
    await phone.exchangeClipboard()
    #expect(phone.clips.first?.text == "later, on the Mac")
    #expect(board.reads == 2) // not read again: nothing was copied on the phone
    #expect(SharingMac.handed.isEmpty)

    phone.clearClips()
    #expect(phone.clips.isEmpty && shelf.clips().isEmpty)
}
