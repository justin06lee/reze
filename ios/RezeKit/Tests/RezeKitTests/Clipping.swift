import Foundation
import Testing

@testable import RezeKit

// The phone's clipboard list, and evening it out with a Mac.

private func scratch() -> Shelf {
    Shelf(root: FileManager.default.temporaryDirectory.appendingPathComponent("rezeclips-\(UUID().uuidString)"))
}

private func macClip(text: String) -> Data {
    try! JSONSerialization.data(withJSONObject: ["kind": "text", "text": text, "mime": "text/plain", "bytes": text.utf8.count])
}

@Test func theSameThingCopiedTwiceIsKeptOnce() throws {
    let shelf = scratch()
    defer { shelf.empty() }

    let blank = try shelf.record(.text("   \n"), from: "", shared: false)
    #expect(blank == nil)
    let made = try shelf.record(.text("one"), from: "", shared: false)
    let first = try #require(made)
    #expect(first.new && first.entry.mine && !first.entry.shared)
    _ = try shelf.record(.text("two"), from: "mini", shared: true)
    #expect(shelf.clips().map(\.text) == ["two", "one"])

    // Copied again on the phone: the same entry, back on top.
    let remade = try shelf.record(.text("one"), from: "", shared: false)
    let again = try #require(remade)
    #expect(!again.new && again.entry.id == first.entry.id)
    #expect(shelf.clips().map(\.text) == ["one", "two"])
    // Arriving again from the Mac: left where it is.
    let echoed = try shelf.record(.text("two"), from: "mini", shared: true)
    #expect(echoed?.new == false)
    #expect(shelf.clips().map(\.text) == ["one", "two"])
}

@Test func anImageIsKeptAsAFileAndGoesWhenItFallsOffTheEnd() throws {
    let shelf = scratch()
    defer { shelf.empty() }

    let kept = try shelf.record(.image(Data([1, 2, 3]), mime: "image/png"), from: "", shared: false)
    let image = try #require(kept).entry
    let url = try #require(shelf.url(for: image))
    #expect(image.file.hasSuffix(".png") && image.bytes == 3)
    let onDisk = try Data(contentsOf: url)
    #expect(onDisk == Data([1, 2, 3]))

    for n in 0..<Shelf.clipLimit { _ = try shelf.record(.text("text \(n)"), from: "", shared: false) }
    #expect(shelf.clips().count == Shelf.clipLimit)
    #expect(!FileManager.default.fileExists(atPath: url.path))

    // A name that is a path has nowhere to be.
    var hostile = image
    hostile.file = "../library.json"
    #expect(shelf.url(for: hostile) == nil)

    shelf.clearClips()
    #expect(shelf.clips().isEmpty)
}

@Test func thePhoneAndTheMacAreEvenedOut() async throws {
    let host = "evens-out"
    PretendMac.set(host) {
        $0.paths = ["/clip": macClip(text: "copied on the Mac")]
        $0.posts = 200
    }
    let shelf = scratch()
    defer { shelf.empty() }
    let client = PretendMac.client(host)

    let first = await shelf.exchange(with: client, copied: .text("copied on the phone"), name: "Justin\u{2019}s iPhone 📱")
    #expect(first.sharing == true && first.trouble == nil)
    #expect(first.arrived?.text == "copied on the Mac" && first.arrived?.from == host)
    #expect(first.took?.text == "copied on the phone")
    #expect(first.sent)
    // The phone's own copy on top, and both now known to the Mac.
    #expect(shelf.clips().map(\.text) == ["copied on the phone", "copied on the Mac"])
    #expect(shelf.clips().filter { !$0.shared }.isEmpty)

    let handed = PretendMac.handed(host)
    #expect(handed.count == 1)
    #expect(handed[0].path == "/clip" && handed[0].type.hasPrefix("text/plain"))
    #expect(String(decoding: handed[0].body, as: UTF8.self) == "copied on the phone")
    // A header carries what ASCII there is of the name.
    #expect(handed[0].from == "Justins iPhone ")

    // The Mac now says its newest copy is the one it was handed: nothing new,
    // and nothing handed over twice.
    PretendMac.set(host) { $0.paths["/clip"] = macClip(text: "copied on the phone") }
    let second = await shelf.exchange(with: client, copied: nil, name: "iPhone")
    #expect(second.arrived == nil && second.took == nil && !second.sent)
    #expect(PretendMac.handed(host).count == 1)
    #expect(shelf.clips().count == 2)
}

@Test func aCopyMadeWhileTheMacIsAwayIsHandedOverLater() async throws {
    let shelf = scratch()
    defer { shelf.empty() }
    PretendMac.set("back-again") { $0.posts = 200; $0.paths = ["/clip": macClip(text: "from the Mac")] }
    _ = await shelf.exchange(with: PretendMac.client("back-again"), copied: nil, name: "iPhone")

    // Nothing answers: the copy is kept all the same.
    let away = await shelf.exchange(with: PretendMac.client("nobody-there"), copied: .text("kept"), name: "iPhone")
    #expect(away.sharing == nil && away.trouble != nil && !away.sent)
    #expect(shelf.clips().first?.shared == false)

    // The Mac is back with nothing new of its own: it gets the copy, without
    // the copy being made again.
    let back = await shelf.exchange(with: PretendMac.client("back-again"), copied: nil, name: "iPhone")
    #expect(back.sent)
    let later = try #require(PretendMac.handed("back-again").first)
    #expect(String(decoding: later.body, as: UTF8.self) == "kept")
}

@Test func aStaleCopyDoesNotReplaceWhatTheMacCopiedSince() async throws {
    let shelf = scratch()
    defer { shelf.empty() }
    _ = await shelf.exchange(with: PretendMac.client("nobody-there"), copied: .text("old, from the phone"), name: "iPhone")

    // By the time the Mac is reached it has copied something else. Which was
    // later cannot be known, and the Mac's clipboard is the one in use.
    PretendMac.set("moved-on") { $0.posts = 200; $0.paths = ["/clip": macClip(text: "newer, on the Mac")] }
    let met = await shelf.exchange(with: PretendMac.client("moved-on"), copied: nil, name: "iPhone")
    #expect(met.arrived != nil && !met.sent)
    #expect(PretendMac.handed("moved-on").isEmpty)
}

@Test func aMacThatDoesNotShareIsNotHandedAnything() async throws {
    let shelf = scratch()
    defer { shelf.empty() }

    PretendMac.set("private") { _ in }
    let quiet = await shelf.exchange(with: PretendMac.client("private"), copied: .text("mine"), name: "iPhone")
    #expect(quiet.sharing == false && !quiet.sent && quiet.trouble == nil)
    #expect(PretendMac.handed("private").isEmpty)
    #expect(shelf.clips().map(\.text) == ["mine"])

    // One that shares but predates being handed things says so, once asked.
    PretendMac.set("old-build") { $0.paths = ["/clip": macClip(text: "x")]; $0.posts = 400 }
    let old = await shelf.exchange(with: PretendMac.client("old-build"), copied: .text("newer"), name: "iPhone")
    #expect(!old.sent)
    #expect(old.trouble?.contains("too old") == true)
}

@Test func anImageGoesBothWaysAndIsFetchedOnce() async throws {
    let host = "pictures"
    let theirs = Data([7, 7, 7, 7])
    PretendMac.set(host) {
        $0.posts = 200
        $0.paths = [
            "/clip": Data(#"{"kind":"image","file":"shot 1.png","mime":"image/png","bytes":4}"#.utf8),
            "/clip/shot%201.png": theirs,
        ]
    }
    let shelf = scratch()
    defer { shelf.empty() }
    let client = PretendMac.client(host)
    let mine = Data([1, 2, 3, 4, 5])

    let first = await shelf.exchange(with: client, copied: .image(mine, mime: "image/jpeg"), name: "iPhone")
    let arrived = try #require(first.arrived)
    #expect(arrived.isImage && arrived.origin == "shot 1.png")
    let fetched = try Data(contentsOf: #require(shelf.url(for: arrived)))
    #expect(fetched == theirs)
    #expect(first.sent)
    let handed = try #require(PretendMac.handed(host).first)
    #expect(handed.type == "image/jpeg" && handed.body == mine)

    // The same image named again is not fetched again.
    _ = await shelf.exchange(with: client, copied: nil, name: "iPhone")
    #expect(PretendMac.asked(host).filter { $0 == "/clip/shot%201.png" }.count == 1)

    // The Mac hands back the phone's own picture under a name of its own:
    // fetched once to find that out, and never after.
    PretendMac.set(host) {
        $0.paths["/clip"] = Data(#"{"kind":"image","file":"theirs.jpg","mime":"image/jpeg","bytes":5}"#.utf8)
        $0.paths["/clip/theirs.jpg"] = mine
    }
    for _ in 0..<2 {
        let echo = await shelf.exchange(with: client, copied: nil, name: "iPhone")
        #expect(echo.arrived == nil && !echo.sent)
    }
    #expect(PretendMac.asked(host).filter { $0 == "/clip/theirs.jpg" }.count == 1)
    #expect(shelf.clips().count == 2)
}
