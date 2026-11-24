import RezeKit
import SwiftUI

/// The library: search it, tap a macro to copy it.
struct MacrosView: View {
    @EnvironmentObject private var phone: Phone
    @Environment(\.colorScheme) private var scheme
    @State private var query = ""
    @State private var filling: Macro?
    @State private var settings = false

    private var results: [Macro] { Fuzzy.search(query, in: phone.macros) }

    var body: some View {
        NavigationStack {
            List {
                if let trouble = phone.trouble {
                    Section {
                        Label(trouble, systemImage: "wifi.exclamationmark")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    } footer: {
                        Text("Showing what was fetched \(lastLooked).")
                    }
                }

                if query.isEmpty, phone.fetched?.clipboard == true {
                    Section {
                        Button {
                            Task { await phone.takeClipboard() }
                        } label: {
                            Label {
                                VStack(alignment: .leading, spacing: 2) {
                                    Text("\(phone.fetched?.mac.label ?? "The Mac")\u{2019}s clipboard")
                                        .foregroundStyle(.primary)
                                    Text("Copy what was last copied there")
                                        .font(.footnote)
                                        .foregroundStyle(.secondary)
                                }
                            } icon: {
                                Image(systemName: "doc.on.clipboard")
                            }
                        }
                    }
                }

                Section {
                    ForEach(results) { macro in
                        Button {
                            use(macro)
                        } label: {
                            MacroRow(macro: macro, shelf: phone.shelf)
                        }
                        .buttonStyle(.plain)
                    }
                } footer: {
                    if !phone.macros.isEmpty, query.isEmpty {
                        Text("\(phone.macros.count) macros from \(phone.fetched?.mac.label ?? "your Mac"), as of \(lastLooked). Tap one to copy it.")
                    }
                }
            }
            .overlay {
                if phone.macros.isEmpty {
                    ContentUnavailableView(
                        "No macros yet",
                        systemImage: "text.badge.plus",
                        description: Text("Add one in Reze on the Mac, then pull down here.")
                    )
                } else if results.isEmpty {
                    ContentUnavailableView.search(text: query)
                }
            }
            .searchable(text: $query, prompt: "Search macros")
            .autocorrectionDisabled()
            #if os(iOS)
            .textInputAutocapitalization(.never)
            #endif
            .refreshable { await phone.refresh() }
            .navigationTitle("Reze")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button {
                        settings = true
                    } label: {
                        if case .refreshing = phone.busy {
                            ProgressView()
                        } else {
                            Label("Settings", systemImage: "gearshape")
                        }
                    }
                }
            }
            .sheet(item: $filling) { macro in
                FillView(macro: macro)
            }
            .sheet(isPresented: $settings) {
                SettingsView()
            }
        }
        .toast($phone.notice)
    }

    private var lastLooked: String {
        guard let at = phone.fetched?.at else { return "earlier" }
        return at.formatted(.relative(presentation: .named))
    }

    /// Copy it straight away if it has nothing to ask; otherwise ask.
    private func use(_ macro: Macro) {
        guard let library = phone.library else { return }
        if Prepared(macro, in: library).variables.isEmpty {
            phone.copy(macro)
        } else {
            filling = macro
        }
    }
}

struct MacroRow: View {
    let macro: Macro
    let shelf: Shelf?

    var body: some View {
        HStack(spacing: 12) {
            MacroBadge(macro: macro, shelf: shelf)
            VStack(alignment: .leading, spacing: 3) {
                Text(macro.name.isEmpty ? "Untitled" : macro.name)
                    .font(.body.weight(.medium))
                    .lineLimit(1)
                Text(macro.summary)
                    .font(.footnote)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            if !macro.attachments.isEmpty, !macro.body.isEmpty {
                Label("\(macro.attachments.count)", systemImage: "paperclip")
                    .labelStyle(.titleAndIcon)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityHint("Copies it")
    }
}
