import RezeKit
import SwiftUI

/// Which Mac, how the copy here stands, how to switch the keyboard on, and
/// the way out.
struct SettingsView: View {
    @EnvironmentObject private var phone: Phone
    @Environment(\.dismiss) private var dismiss
    @State private var forgetting = false

    var body: some View {
        NavigationStack {
            Form {
                if let fetched = phone.fetched {
                    Section {
                        LabeledContent("Mac", value: fetched.mac.label)
                        LabeledContent("Address") {
                            Text(verbatim: fetched.mac.port == Mac.port ? fetched.mac.host : "\(fetched.mac.host):\(fetched.mac.port)")
                                .font(.callout.monospaced())
                        }
                        LabeledContent("Macros", value: "\(phone.macros.count)")
                        LabeledContent("Attachments") {
                            Text("\(phone.library?.attachments.count ?? 0) · \(formatBytes(phone.shelf?.bytes() ?? 0))")
                        }
                        LabeledContent("Last looked", value: fetched.at.formatted(date: .abbreviated, time: .shortened))
                        Button {
                            Task { await phone.refresh() }
                        } label: {
                            HStack {
                                Text(lookLabel)
                                Spacer()
                                if phone.busy != nil { ProgressView() }
                            }
                        }
                        .disabled(phone.busy != nil)
                    } header: {
                        Text("Fetched from")
                    } footer: {
                        Text(phone.trouble ?? "Reze looks again each time it is opened. The phone only reads: nothing here changes the Mac.")
                    }

                    Section {
                        LabeledContent("Its clipboard", value: fetched.clipboard ? "Shared" : "Not shared")
                    } footer: {
                        Text(fetched.clipboard
                            ? "The first row of the list copies whatever was last copied on \(fetched.mac.label). A phone will not let an app watch the clipboard, so it comes when asked for, not by itself."
                            : "Turn on \u{201C}Share the clipboard between my devices\u{201D} in Reze on \(fetched.mac.label) to copy from it here.")
                    }
                }

                Section {
                    step(1, "Open the Settings app → General → Keyboard → Keyboards → Add New Keyboard, and pick Reze.")
                    step(2, "Tap Reze in that list and turn on Allow Full Access if you want macros with pictures or video. Text needs nothing.")
                    step(3, "In any app, hold the globe key and choose Reze. Tap a macro to type it, or type its name first and the keyboard offers to replace it.")
                } header: {
                    Text("The keyboard")
                } footer: {
                    Text("Full Access is what lets a keyboard put a picture on the clipboard. Reze\u{2019}s keyboard never uses the network: it reads the copy this app keeps.")
                }

                Section {
                    Button("Forget this Mac", role: .destructive) { forgetting = true }
                } footer: {
                    Text("Removes the copy from this phone. The Mac\u{2019}s library is untouched.")
                }
            }
            .formStyle(.grouped)
            .navigationTitle("Settings")
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .confirmationAction) {
                    Button("Done") { dismiss() }
                }
            }
            .confirmationDialog(
                "Forget \(phone.fetched?.mac.label ?? "this Mac")?",
                isPresented: $forgetting,
                titleVisibility: .visible
            ) {
                Button("Forget it", role: .destructive) {
                    phone.forget()
                    dismiss()
                }
            } message: {
                Text("Its macros and attachments are removed from this phone, and from the keyboard.")
            }
        }
    }

    private var lookLabel: String {
        switch phone.busy {
        case .refreshing(let done, let of) where of > 0: return "Fetching attachments, \(done) of \(of)…"
        case .some: return "Looking…"
        case .none: return "Look again now"
        }
    }

    private func step(_ number: Int, _ text: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: 10) {
            Text("\(number).")
                .font(.callout.monospacedDigit())
                .foregroundStyle(.secondary)
            Text(text)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}
