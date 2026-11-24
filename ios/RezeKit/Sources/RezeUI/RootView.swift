import RezeKit
import SwiftUI

/// The whole app: the screen that asks which Mac, until there is one, and the
/// library after.
public struct RootView: View {
    @StateObject private var phone: Phone
    @Environment(\.scenePhase) private var phase
    @Environment(\.colorScheme) private var scheme

    public init() {
        _phone = StateObject(wrappedValue: Phone())
    }

    public init(phone: Phone) {
        _phone = StateObject(wrappedValue: phone)
    }

    public var body: some View {
        Group {
            if phone.fetched == nil {
                ConnectView()
            } else {
                MacrosView()
            }
        }
        .environmentObject(phone)
        .tint(Theme.accent(scheme))
        // Each time the app comes to the front, look at the Mac again. A
        // phone gives an app no way to be told, and little time in the
        // background to ask, so this is when the copy is brought up to date.
        .onChange(of: phase, initial: true) { _, now in
            guard now == .active else { return }
            Task {
                // `-RezeMac mac-mini` on the command line names the Mac
                // without typing it: what `make ios-sim MAC=…` passes, and
                // what a scheme's launch arguments can.
                if phone.fetched == nil, let named = UserDefaults.standard.string(forKey: "RezeMac") {
                    await phone.find(named)
                } else {
                    await phone.refresh()
                }
            }
        }
    }
}
