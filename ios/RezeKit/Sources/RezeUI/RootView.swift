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
            if now == .active {
                Task { await phone.refresh() }
            }
        }
    }
}
