import RezeKit
import SwiftUI

/// A macro that asks for something before it is text: one field for each of
/// its variables, and what it will come to as they are filled in.
struct FillView: View {
    let macro: Macro

    @EnvironmentObject private var phone: Phone
    @Environment(\.dismiss) private var dismiss
    @State private var values: [String: String] = [:]
    @FocusState private var focused: String?

    private var prepared: Prepared? {
        phone.library.map { Prepared(macro, in: $0) }
    }

    var body: some View {
        NavigationStack {
            Form {
                if let prepared {
                    Section {
                        ForEach(prepared.variables, id: \.name) { variable in
                            VStack(alignment: .leading, spacing: 4) {
                                Text(variable.name)
                                    .font(.footnote)
                                    .foregroundStyle(.secondary)
                                TextField(
                                    variable.fallback,
                                    text: binding(variable.name),
                                    axis: .vertical
                                )
                                .textFieldStyle(.plain)
                                .lineLimit(1...4)
                                .focused($focused, equals: variable.name)
                            }
                        }
                    } footer: {
                        Text("One left blank takes its default, shown in grey.")
                    }

                    Section("What gets copied") {
                        Text(preview(prepared))
                            .font(.callout.monospaced())
                            .foregroundStyle(.secondary)
                            .lineLimit(12)
                            .textSelection(.enabled)
                    }

                    if !prepared.missing.isEmpty {
                        Section {
                            Label(
                                "It includes \(prepared.missing.joined(separator: ", ")), which is not a macro in this library.",
                                systemImage: "exclamationmark.triangle"
                            )
                            .font(.footnote)
                        }
                    }
                }
            }
            .formStyle(.grouped)
            .navigationTitle(macro.name)
            #if os(iOS)
            .navigationBarTitleDisplayMode(.inline)
            #endif
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Cancel") { dismiss() }
                }
                ToolbarItem(placement: .confirmationAction) {
                    Button("Copy") {
                        phone.copy(macro, values: values)
                        dismiss()
                    }
                    .bold()
                }
            }
            .onAppear { focused = prepared?.variables.first?.name }
        }
        .presentationDetents([.medium, .large])
    }

    private func binding(_ name: String) -> Binding<String> {
        Binding(get: { values[name] ?? "" }, set: { values[name] = $0 })
    }

    /// The text as it stands. The clipboard is not read for a preview — on a
    /// phone that asks permission, and asking on every keystroke would be
    /// worse than showing where it will go.
    private func preview(_ prepared: Prepared) -> String {
        let text = prepared.render(values: values, clipboard: "‹clipboard›")
        return text.isEmpty ? "Nothing but its attachments" : text
    }
}
