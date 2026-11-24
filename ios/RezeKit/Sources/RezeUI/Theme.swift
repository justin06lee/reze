import SwiftUI

/// The desktop app's colours (`src/editor.css`), so the phone reads as the
/// same thing.
public enum Theme {
    /// `--accent`: lighter on dark, deeper on light.
    public static func accent(_ scheme: ColorScheme) -> Color {
        scheme == .dark
            ? Color(red: 1.0, green: 0x7a / 255, blue: 0x8a / 255)
            : Color(red: 0xd1 / 255, green: 0x43 / 255, blue: 0x5a / 255)
    }

    /// What goes on top of the accent: the desktop's buttons use near-black
    /// on the light pink and white on the deep one.
    public static func onAccent(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(white: 0.09) : .white
    }

    /// `--bg-raised`: a card on the page.
    public static func raised(_ scheme: ColorScheme) -> Color {
        scheme == .dark ? Color(white: 1, opacity: 0.07) : Color(white: 0, opacity: 0.05)
    }
}

/// A line that says what just happened and then goes away.
struct Toast: ViewModifier {
    @Binding var message: String?
    @Environment(\.colorScheme) private var scheme

    func body(content: Content) -> some View {
        content.overlay(alignment: .bottom) {
            if let message {
                Text(message)
                    .font(.subheadline.weight(.medium))
                    .multilineTextAlignment(.center)
                    .padding(.horizontal, 16)
                    .padding(.vertical, 10)
                    .background(.regularMaterial, in: Capsule())
                    .overlay(Capsule().strokeBorder(Theme.accent(scheme).opacity(0.5)))
                    .padding(.horizontal, 24)
                    .padding(.bottom, 20)
                    .transition(.move(edge: .bottom).combined(with: .opacity))
                    .task(id: message) {
                        try? await Task.sleep(for: .seconds(2.5))
                        if !Task.isCancelled { self.message = nil }
                    }
                    .accessibilityAddTraits(.isStaticText)
            }
        }
        .animation(.snappy, value: message)
    }
}

extension View {
    func toast(_ message: Binding<String?>) -> some View {
        modifier(Toast(message: message))
    }
}
