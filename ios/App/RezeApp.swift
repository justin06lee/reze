import RezeUI
import SwiftUI

// The app is its screens, and its screens are in RezeUI (`ios/RezeKit`),
// where they build and are tested on a Mac. This is only the door.

@main
struct RezeApp: App {
    var body: some Scene {
        WindowGroup {
            RootView()
        }
    }
}
