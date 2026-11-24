import RezeKit
import SwiftUI

/// The first screen: which Mac to fetch from.
///
/// A phone cannot ask Tailscale who else is on the tailnet the way a Mac
/// can, so the Mac is typed in once — by the name or address its own
/// Settings → Other devices shows.
struct ConnectView: View {
    @EnvironmentObject private var phone: Phone
    @Environment(\.colorScheme) private var scheme
    @State private var typed = ""
    @FocusState private var typing: Bool

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 22) {
                VStack(alignment: .leading, spacing: 8) {
                    Text("Your macros, on your phone.")
                        .font(.largeTitle.bold())
                    Text("Reze on this phone keeps a copy of the library on one of your Macs, fetched over Tailscale.")
                        .font(.body)
                        .foregroundStyle(.secondary)
                }

                VStack(alignment: .leading, spacing: 14) {
                    step(1, "Tailscale is on, on this phone", "Signed in as the same person as the Mac. That is the only thing the Mac checks.")
                    step(2, "Reze is running on the Mac", "With \u{201C}Let my other devices import from this one\u{201D} on, in Settings → Other devices.")
                    step(3, "Type the Mac\u{2019}s name", "The same screen on the Mac says what it is called, like mac-mini, and its 100.x.y.z address. Either works.")
                }

                VStack(alignment: .leading, spacing: 10) {
                    TextField("mac-mini", text: $typed)
                        .textFieldStyle(.plain)
                        .font(.body.monospaced())
                        .autocorrectionDisabled()
                        #if os(iOS)
                        .textInputAutocapitalization(.never)
                        .keyboardType(.URL)
                        .submitLabel(.go)
                        #endif
                        .focused($typing)
                        .onSubmit(find)
                        .padding(.horizontal, 14)
                        .padding(.vertical, 12)
                        .background(Theme.raised(scheme), in: RoundedRectangle(cornerRadius: 12, style: .continuous))

                    if let trouble = phone.trouble {
                        Text(trouble)
                            .font(.footnote)
                            .foregroundStyle(.red)
                            .fixedSize(horizontal: false, vertical: true)
                    }

                    Button(action: find) {
                        HStack(spacing: 8) {
                            if phone.busy != nil { ProgressView().controlSize(.small) }
                            Text(phone.busy == nil ? "Fetch my macros" : "Looking for it…")
                                .font(.headline)
                        }
                        .frame(maxWidth: .infinity)
                        .padding(.vertical, 13)
                        .foregroundStyle(Theme.onAccent(scheme))
                        .background(Theme.accent(scheme), in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                    }
                    .buttonStyle(.plain)
                    .disabled(phone.busy != nil || typed.trimmingCharacters(in: .whitespaces).isEmpty)
                    .opacity(typed.trimmingCharacters(in: .whitespaces).isEmpty ? 0.5 : 1)
                }

                Text("Nothing on the Mac is changed from here. The phone only reads.")
                    .font(.footnote)
                    .foregroundStyle(.tertiary)
            }
            .padding(24)
            .frame(maxWidth: 560, alignment: .leading)
            .frame(maxWidth: .infinity)
        }
        .scrollDismissesKeyboard(.interactively)
    }

    private func find() {
        typing = false
        Task { await phone.find(typed) }
    }

    private func step(_ number: Int, _ title: String, _ detail: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 12) {
            Text("\(number)")
                .font(.footnote.bold().monospacedDigit())
                .foregroundStyle(Theme.accent(scheme))
                .frame(width: 22, height: 22)
                .background(Theme.accent(scheme).opacity(0.15), in: Circle())
            VStack(alignment: .leading, spacing: 2) {
                Text(title).font(.headline)
                Text(detail)
                    .font(.subheadline)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}
