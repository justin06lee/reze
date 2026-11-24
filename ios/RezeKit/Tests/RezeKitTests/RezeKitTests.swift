import Foundation
import Testing

@testable import RezeKit

// MARK: the library as a Mac hands it over

@Test func aLibraryIsReadWhateverElseIsInIt() throws {
    // A real `macros.json` carries settings, remembered deletions and whatever
    // the desktop adds next; a macro may predate fields that exist now.
    let raw = """
    {
      "version": 1,
      "settings": { "hotkey": "CmdOrCtrl+Shift+Space", "folders": ["/Users/x/.reze/media"], "sync": true },
      "deleted": { "gone": 1794965935068 },
      "somethingNew": { "nested": [1, 2, 3] },
      "macros": [
        { "id": "a", "name": "old one", "body": "hi" },
        {
          "id": "b", "name": "meme", "description": "", "tags": ["fun"], "body": "",
          "attachments": [
            { "id": "1", "file": "x.mp4", "name": "Untitled.mp4", "kind": "video", "mime": "video/mp4", "bytes": 3370611 },
            { "id": "2", "file": "", "path": "/Users/x/.reze/media/y.png", "name": "y.png", "kind": "image", "mime": "image/png", "bytes": 5 },
            { "id": "3", "file": "z.bin", "name": "z", "kind": "hologram", "mime": "x/y", "bytes": 1, "extra": true }
          ],
          "usageCount": 3, "updatedAt": 1794965935068, "futureField": "ignored"
        }
      ]
    }
    """
    let library = try JSONDecoder().decode(Library.self, from: Data(raw.utf8))

    #expect(library.macros.map(\.name) == ["old one", "meme"])
    #expect(library.macros[0].attachments.isEmpty)
    #expect(library.macros[0].usageCount == 0)
    #expect(library.macros[1].updatedAt == 1_794_965_935_068)
    #expect(library.macros[1].attachments.map(\.kind) == [.video, .image, .file])
    // The one the Mac references in a watched folder is not one it serves.
    #expect(library.attachments.map(\.file) == ["x.mp4", "z.bin"])
}

@Test func aHelloFromAnOlderBuildIsStillAHello() throws {
    let hello = try JSONDecoder().decode(Hello.self, from: Data(#"{"protocol":1}"#.utf8))
    #expect(hello.protocol == 1)
    #expect(!hello.sync && !hello.clipboard && hello.revision.isEmpty)
}

@Test func sizesReadAsFinderShowsThem() {
    #expect(formatBytes(999) == "999 B")
    #expect(formatBytes(1500) == "1.5 kB")
    #expect(formatBytes(3_370_611) == "3.4 MB")
    #expect(formatBytes(25_000_000) == "25 MB")
}

// MARK: making text of a macro

@Test func aMacroIsPreparedAndRendered() {
    let library = Library(macros: [
        Macro(name: "sig", body: "— {{author|me}}"),
        Macro(name: "note", body: "{{greeting|Hi}} {{who}},\n{{clipboard}}\n{{> sig}} {{date}}"),
    ])
    let prepared = Prepared(library.macros[1], in: library)

    #expect(prepared.variables.map(\.name) == ["greeting", "who", "author"])
    #expect(prepared.wantsClipboard)
    #expect(prepared.missing.isEmpty && prepared.cyclic.isEmpty)

    let now = Date(timeIntervalSince1970: 1_795_000_000) // 2026-11-18T11:06:40Z
    #expect(
        prepared.render(values: ["who": "Sam"], clipboard: "pasted", now: now)
            == "Hi Sam,\npasted\n— me 2026-11-18"
    )
    // Nothing typed in can stand in for a built-in.
    #expect(prepared.render(values: ["date": "never"], now: now).hasSuffix("2026-11-18"))
}

@Test func builtInsTakeTheFormsTheDesktopGivesThem() throws {
    let now = Date(timeIntervalSince1970: 1_795_000_000.123)
    let tokyo = try #require(TimeZone(identifier: "Asia/Tokyo"))
    let values = Template.builtinValues(Builtin.allCases, clipboard: "c", now: now, zone: tokyo)
    #expect(values["clipboard"] == "c")
    #expect(values["date"] == "2026-11-18")
    #expect(values["time"] == "20:06")
    #expect(values["datetime"] == "2026-11-18T11:06:40.123Z")
}

// MARK: finding a macro

private let names = ["analysis", "full analysis", "commit message", "rigor"].map { Macro(name: $0) }

@Test func searchingRanksLikeThePalette() {
    let macros = [
        Macro(name: "refactor plan", usageCount: 9),
        Macro(name: "full analysis", usageCount: 1),
        Macro(name: "zebra", tags: ["fauna"]),
    ]
    #expect(Fuzzy.search("fa", in: macros).map(\.name) == ["full analysis", "refactor plan", "zebra"])
    #expect(Fuzzy.search("", in: macros).map(\.name) == ["refactor plan", "full analysis", "zebra"])
    #expect(Fuzzy.search("qqq", in: macros).isEmpty)
}

@Test func theLongestTriggerJustTypedIsTheOneMeant() throws {
    let trigger = try #require(Triggers.match("please do a full analysis", in: names))
    #expect(trigger.macro.name == "full analysis")
    #expect(trigger.delete == "full analysis".count)
    #expect(trigger.trailing.isEmpty)
}

@Test func aTriggerKeepsTheSpacesTypedAfterIt() throws {
    let trigger = try #require(Triggers.match("so: Commit   Message \n", in: names))
    #expect(trigger.macro.name == "commit message")
    #expect(trigger.delete == "Commit   Message \n".count)
    #expect(trigger.trailing == " \n")
}

@Test func halfAWordIsNotATrigger() {
    #expect(Triggers.match("psychoanalysis", in: names) == nil)
    #expect(Triggers.match("rigor mortis", in: names) == nil)
    #expect(Triggers.match("   ", in: names) == nil)
    #expect(Triggers.match("", in: names) == nil)
    #expect(Triggers.match("anything", in: [Macro(name: "  ")]) == nil)
}

@Test func aTriggerCountsCharactersAsTheKeyboardDeletesThem() throws {
    // One tap of delete takes a whole emoji, so that is how they are counted.
    let trigger = try #require(Triggers.match("👨‍👩‍👧 café rigor", in: names))
    #expect(trigger.delete == 5)
}

// MARK: naming a Mac

@Test func aMacIsReadHoweverItIsTyped() {
    #expect(Mac(typed: "mac-mini") == Mac(host: "mac-mini"))
    #expect(Mac(typed: "  Mac-Mini.tail1234.ts.net.  ") == Mac(host: "mac-mini.tail1234.ts.net"))
    #expect(Mac(typed: "100.71.90.33") == Mac(host: "100.71.90.33"))
    #expect(Mac(typed: "http://100.71.90.33:7393/reze") == Mac(host: "100.71.90.33"))
    #expect(Mac(typed: "mini:8080") == Mac(host: "mini", port: 8080))

    for nonsense in ["", "   ", "my mac", "mini:", "mini:0", "mini:99999", "a..b", ".mini", "mini/../x@evil", "exa_mple"] {
        #expect(Mac(typed: nonsense) == nil || nonsense == "mini/../x@evil", "\(nonsense)")
    }
    // A path is dropped, not followed.
    #expect(Mac(typed: "mini/../x@evil") == Mac(host: "mini"))

    #expect(Mac(host: "mac-mini.tail1234.ts.net").label == "mac-mini")
    #expect(Mac(host: "100.71.90.33").label == "100.71.90.33")
}

@Test func fileNamesAreEscapedAsTheMacEscapesThem() {
    #expect(Client.encode("7061b8c7-90fe.mp4") == "7061b8c7-90fe.mp4")
    #expect(Client.encode("a b/c%é~_.png") == "a%20b%2Fc%25%C3%A9~_.png")
}
