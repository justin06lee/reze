import XCTest

// The app and its keyboard, driven the way a person would, in a simulator,
// against a real Mac's Reze:
//
//   make ios-sim && make ios-uitest MAC=mac-mini      (or ⌘U in Xcode, with
//   REZE_LIVE set in the scheme's test environment)
//
// A simulated phone is on the Mac's own network, so it reaches the Mac's
// Tailscale address the way a phone on the tailnet would. Nothing on the Mac
// changes: the app only reads.
//
// The tests run in name order and the second leans on the first having
// fetched a library. The second also needs Reze's keyboard added in the
// simulator's Settings, as on a phone, and is skipped without it.
//
// These compile and have not run yet: see README, "What has not been tried".

final class RezeFlowTests: XCTestCase {
    private let app = XCUIApplication()

    /// The Mac to fetch from. `TEST_RUNNER_REZE_LIVE` outside is `REZE_LIVE` here.
    private var mac: String? { ProcessInfo.processInfo.environment["REZE_LIVE"] }

    override func setUp() {
        continueAfterFailure = false
    }

    /// Keep a picture of the screen. A simulated phone's processes can write
    /// to the Mac's disk, which is the simple way to get one out.
    private func shoot(_ name: String) {
        let picture = XCUIScreen.main.screenshot()
        let kept = XCTAttachment(screenshot: picture)
        kept.name = name
        kept.lifetime = .keepAlways
        add(kept)
        if let folder = ProcessInfo.processInfo.environment["REZE_SHOTS"] {
            try? FileManager.default.createDirectory(atPath: folder, withIntermediateDirectories: true)
            try? picture.pngRepresentation.write(to: URL(fileURLWithPath: folder).appendingPathComponent("\(name).png"))
        }
    }

    private var macros: XCUIElementQuery {
        app.descendants(matching: .any).matching(identifier: "reze.macro")
    }

    private var notice: XCUIElement {
        app.descendants(matching: .any).matching(identifier: "reze.notice").firstMatch
    }

    func test1FetchesAMacsLibraryAndCopiesAMacro() throws {
        let mac = try XCTUnwrap(mac, "name a Mac: make ios-uitest MAC=mac-mini")
        app.launch()

        // Start from nothing, whatever an earlier run left behind.
        if app.navigationBars["Reze"].waitForExistence(timeout: 5) {
            app.buttons["Settings"].tap()
            let forget = app.buttons["Forget this Mac"]
            XCTAssertTrue(forget.waitForExistence(timeout: 5))
            while !forget.isHittable { app.swipeUp() }
            forget.tap()
            app.buttons["Forget it"].firstMatch.tap()
        }

        let field = app.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 10), "the screen that asks which Mac")
        shoot("1-which-mac")

        // Something that is not a machine is said to be, and nothing is fetched.
        field.tap()
        field.typeText("not a mac!\n")
        XCTAssertTrue(
            app.staticTexts.matching(NSPredicate(format: "label CONTAINS 'name or address'")).firstMatch
                .waitForExistence(timeout: 5)
        )

        field.tap()
        field.press(forDuration: 1.2)
        if app.menuItems["Select All"].waitForExistence(timeout: 2) { app.menuItems["Select All"].tap() }
        field.typeText(mac)
        app.buttons["Fetch my macros"].tap()

        XCTAssertTrue(app.navigationBars["Reze"].waitForExistence(timeout: 60), "the library, fetched from \(mac)")
        XCTAssertTrue(macros.firstMatch.waitForExistence(timeout: 10), "its macros, listed")
        XCTAssertGreaterThan(macros.count, 0)
        // Long enough for the pictures to be fetched and drawn.
        sleep(3)
        shoot("2-macros")

        // Tapping one either copies it or asks for what it needs first.
        macros.firstMatch.tap()
        let copy = app.buttons["Copy"]
        if copy.waitForExistence(timeout: 2) {
            shoot("3-fill")
            copy.tap()
        }
        XCTAssertTrue(notice.waitForExistence(timeout: 5), "it says what it did")
        XCTAssertTrue(notice.label.hasPrefix("Copied") || notice.label.contains("empty"), notice.label)
        shoot("3-copied")

        app.buttons["Settings"].tap()
        XCTAssertTrue(app.staticTexts["Fetched from"].waitForExistence(timeout: 5) || app.staticTexts["FETCHED FROM"].exists)
        shoot("4-settings")
    }

    func test2TheKeyboardTypesAMacro() throws {
        app.launch()
        XCTAssertTrue(app.navigationBars["Reze"].waitForExistence(timeout: 10), "run after the library is fetched")

        // The search field is a text field like any other app's.
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        search.tap()

        // Reze's own delete key is a button; the system keyboard's is a key.
        let ours = app.buttons["Delete"]
        var tries = 0
        while !ours.exists, tries < 6 {
            let globe = app.buttons["Next keyboard"].firstMatch
            guard globe.waitForExistence(timeout: 5) else { break }
            globe.tap()
            _ = ours.waitForExistence(timeout: 2)
            tries += 1
        }
        try XCTSkipUnless(
            ours.exists,
            "Reze's keyboard is not among this simulator's keyboards: add it in its Settings"
        )
        sleep(1)
        shoot("5-keyboard")

        let keys = app.descendants(matching: .any).matching(identifier: "reze.key")
        XCTAssertTrue(keys.firstMatch.waitForExistence(timeout: 5), "the macros, as keys")
        let before = search.value as? String ?? ""
        keys.firstMatch.tap()
        sleep(1)
        let after = search.value as? String ?? ""
        let said = notice.exists ? notice.label : ""
        XCTAssertTrue(after != before || !said.isEmpty, "tapping a macro types it, or says why it could not")
        shoot("6-typed")

        // And the keys a keyboard cannot go without.
        app.buttons["Delete"].tap()
        XCTAssertTrue(app.buttons["Return"].exists)
    }
}
