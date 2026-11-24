import Foundation
import Testing

import RezeKit
@testable import RezeUI

// What the keyboard does to a text field, with a text field made of a string.
// None of these reach the clipboard: the one on the Mac running the tests is
// somebody's, and a test has no business replacing what is on it.

@MainActor
private final class Field {
    var text: String
    var fullAccess = false

    init(_ text: String) {
        self.text = text
    }

    var proxy: KeyboardState.Proxy {
        KeyboardState.Proxy(
            insert: { self.text += $0 },
            deleteBackward: { _ = self.text.popLast() },
            nextKeyboard: {},
            context: { self.text },
            fullAccess: { self.fullAccess },
            needsGlobe: { false }
        )
    }
}

@MainActor
private func keyboard(_ field: Field, _ macros: [Macro]) throws -> (KeyboardState, Shelf) {
    let shelf = Shelf(root: FileManager.default.temporaryDirectory.appendingPathComponent("rezeui-\(UUID().uuidString)"))
    try shelf.keep(Library(macros: macros))
    let state = KeyboardState(shelf: shelf, proxy: field.proxy)
    state.reload()
    return (state, shelf)
}

private let library = [
    Macro(name: "analysis", body: "ANALYSIS", usageCount: 1),
    Macro(name: "full analysis", tags: ["prompts"], body: "Analyse {{topic|everything}} fully.", usageCount: 5),
    Macro(name: "blank", body: "Dear {{who}},"),
    Macro(name: "pasted", body: "[{{clipboard}}]"),
    Macro(name: "meme", tags: ["fun"], attachments: [Attachment(file: "m.png", name: "meme.png", kind: .image, mime: "image/png", bytes: 3)]),
]

@MainActor
@Test func tappingAMacroTypesItWithItsDefaults() throws {
    let field = Field("So: ")
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }

    #expect(state.trigger == nil)
    #expect(state.macros.first?.name == "full analysis") // the most used first
    state.type(library[1])
    #expect(field.text == "So: Analyse everything fully.")
    #expect(state.notice == nil)
    #expect(state.context == field.text)
}

@MainActor
@Test func aNameJustTypedIsReplacedByItsMacro() throws {
    let field = Field("please do a Full  Analysis ")
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }

    let trigger = try #require(state.trigger)
    #expect(trigger.macro.name == "full analysis") // not "analysis": the longest wins
    state.type(trigger.macro, replacing: trigger)
    // The name is gone, the space typed after it is kept.
    #expect(field.text == "please do a Analyse everything fully. ")
    #expect(state.trigger == nil)
}

@MainActor
@Test func whatAKeyboardCannotDoIsSaid() throws {
    let field = Field("")
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }

    state.type(library[2])
    #expect(field.text == "Dear ,")
    #expect(state.notice?.contains("blanks") == true)

    field.text = ""
    state.type(library[3])
    #expect(field.text == "[]")
    #expect(state.notice?.contains("Full Access") == true)

    // A picture, with no Full Access: nothing is typed, and it says why.
    field.text = "meme"
    state.look()
    let trigger = try #require(state.trigger)
    state.type(trigger.macro, replacing: trigger)
    #expect(field.text.isEmpty)
    #expect(state.notice?.contains("Full Access") == true)
}

@MainActor
@Test func aPictureNotFetchedYetIsNotCopied() throws {
    let field = Field("")
    field.fullAccess = true
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }

    // Full Access, but the file never arrived: the clipboard is left alone.
    state.type(library[4])
    #expect(state.notice?.contains("Not fetched yet") == true)
}

@MainActor
@Test func tagsNarrowWhatIsOnOffer() throws {
    let field = Field("")
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }

    #expect(state.tags == ["fun", "prompts"])
    state.tag = "fun"
    #expect(state.macros.map(\.name) == ["meme"])

    // The tag goes away with the last macro that had it.
    try shelf.keep(Library(macros: Array(library.prefix(4))))
    state.reload()
    #expect(state.tag == nil)
    #expect(state.macros.count == 4)
}

@MainActor
@Test func theKeysTypeWhatTheySay() throws {
    let field = Field("ab")
    let (state, shelf) = try keyboard(field, library)
    defer { shelf.empty() }
    state.space()
    state.newline()
    #expect(field.text == "ab \n")
    state.delete()
    state.delete()
    #expect(field.text == "ab")
}

@MainActor
@Test func aMacThatDoesNotReadAsOneIsNotLookedFor() async throws {
    let shelf = Shelf(root: FileManager.default.temporaryDirectory.appendingPathComponent("rezeui-\(UUID().uuidString)"))
    defer { shelf.empty() }
    let phone = Phone(shelf: shelf)
    #expect(await phone.find("my mac!") == false)
    #expect(phone.trouble?.contains("name or address") == true)
    #expect(phone.fetched == nil)

    let nowhere = Phone(shelf: nil)
    #expect(await nowhere.find("mac-mini") == false)
    #expect(nowhere.trouble?.contains("App Group") == true)
}
