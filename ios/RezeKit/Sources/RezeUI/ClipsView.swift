import RezeKit
import SwiftUI

/// The clipboard: what was copied on this phone and on the Mac, newest first.
/// Tap one to put it back on the phone's clipboard.
struct ClipsView: View {
    @EnvironmentObject private var phone: Phone
    @State private var clearing = false

    var body: some View {
        NavigationStack {
            List {
                if let said = banner {
                    Section {
                        Label(said, systemImage: phone.sharing ? "wifi.exclamationmark" : "doc.on.clipboard")
                            .font(.footnote)
                            .foregroundStyle(.secondary)
                    }
                }
                Section {
                    ForEach(phone.clips) { clip in
                        Button {
                            phone.copy(clip)
                        } label: {
                            ClipRow(clip: clip, shelf: phone.shelf)
                        }
                        .buttonStyle(.plain)
                    }
                } footer: {
                    if !phone.clips.isEmpty {
                        Text("Tap one to copy it. Reze looks at the clipboard when it is opened and when its keyboard is up: a phone lets nothing watch it in between.")
                    }
                }
            }
            .overlay {
                if phone.clips.isEmpty {
                    ContentUnavailableView(
                        "Nothing copied yet",
                        systemImage: "doc.on.clipboard",
                        description: Text("Copy something on this phone or on \(phone.fetched?.mac.label ?? "the Mac"), then come back here.")
                    )
                }
            }
            .refreshable { await phone.exchangeClipboard() }
            .navigationTitle("Clipboard")
            .toolbar {
                ToolbarItem(placement: .primaryAction) {
                    Button("Clear") { clearing = true }
                        .disabled(phone.clips.isEmpty)
                }
            }
            .confirmationDialog("Clear the clipboard list?", isPresented: $clearing, titleVisibility: .visible) {
                Button("Clear it", role: .destructive) { phone.clearClips() }
            } message: {
                Text("Removes these from this phone. The Mac\u{2019}s own history is untouched.")
            }
        }
        .toast($phone.notice)
    }

    /// What is in the way of the two clipboards being one, if anything is.
    private var banner: String? {
        let mac = phone.fetched?.mac.label ?? "the Mac"
        if !phone.sharing, phone.clipTrouble == nil {
            return "\(mac) is not sharing its clipboard, so this list is this phone\u{2019}s alone. On \(mac): Reze → Settings → Clipboard history → \u{201C}Share the clipboard between my devices\u{201D}."
        }
        return phone.clipTrouble
    }
}

/// One copy: what it is, where it was copied, and when.
struct ClipRow: View {
    let clip: ClipEntry
    let shelf: Shelf?
    /// Tighter, for the keyboard.
    var compact = false

    var body: some View {
        HStack(spacing: compact ? 8 : 12) {
            if clip.isImage {
                Thumbnail(
                    attachment: Attachment(file: clip.file, kind: .image, mime: clip.mime, bytes: clip.bytes),
                    url: shelf?.url(for: clip),
                    side: compact ? 32 : 44
                )
            }
            VStack(alignment: .leading, spacing: 3) {
                Text(clip.isImage ? "Image · \(formatBytes(clip.bytes))" : clip.text.trimmingCharacters(in: .whitespacesAndNewlines))
                    .font(compact ? .subheadline : .body)
                    .lineLimit(compact ? 1 : 3)
                    .multilineTextAlignment(.leading)
                Text(caption)
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 0)
        }
        .contentShape(Rectangle())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("reze.clip")
    }

    private var caption: String {
        let place = clip.mine ? "This phone" : clip.from
        return "\(place) · \(clip.at.formatted(.relative(presentation: .named)))"
    }
}
