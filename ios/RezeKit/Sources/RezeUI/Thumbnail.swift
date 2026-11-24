import ImageIO
import RezeKit
import SwiftUI

/// A small picture of an attachment: the image itself, or a glyph for what
/// kind of file it is.
///
/// Images are read at the size they are drawn, never whole. The keyboard is
/// given a few tens of megabytes to live in and is closed without a word if
/// it goes over, which decoding a handful of photographs would do.
struct Thumbnail: View {
    let attachment: Attachment
    let url: URL?
    var side: CGFloat = 44

    @State private var image: CGImage?
    @Environment(\.displayScale) private var scale
    @Environment(\.colorScheme) private var scheme

    var body: some View {
        ZStack {
            RoundedRectangle(cornerRadius: side * 0.22, style: .continuous)
                .fill(Theme.raised(scheme))
            if let image {
                Image(decorative: image, scale: scale)
                    .resizable()
                    .scaledToFill()
            } else {
                Image(systemName: glyph)
                    .font(.system(size: side * 0.42))
                    .foregroundStyle(.secondary)
            }
        }
        .frame(width: side, height: side)
        .clipShape(RoundedRectangle(cornerRadius: side * 0.22, style: .continuous))
        .task(id: url) {
            guard attachment.kind == .image, let url else { return }
            let pixels = Int(side * scale)
            image = await Task.detached(priority: .utility) { Thumbnails.read(url, pixels: pixels) }.value
        }
    }

    private var glyph: String {
        switch attachment.kind {
        case .image: return "photo"
        case .video: return "film"
        case .audio: return "waveform"
        case .file: return "doc"
        }
    }
}

/// Reading images small. Apart from the view so that it can be done off the
/// main thread, which decoding even a small picture should be.
enum Thumbnails {
    private static let cache: NSCache<NSString, CGImage> = {
        let cache = NSCache<NSString, CGImage>()
        cache.countLimit = 60
        return cache
    }()

    /// The image at `url`, no bigger than `pixels` on its longer side.
    static func read(_ url: URL, pixels: Int) -> CGImage? {
        let key = "\(url.path)@\(pixels)" as NSString
        if let kept = cache.object(forKey: key) { return kept }
        let whole = [kCGImageSourceShouldCache: false] as CFDictionary
        guard let source = CGImageSourceCreateWithURL(url as CFURL, whole) else { return nil }
        let small = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceShouldCacheImmediately: true,
            kCGImageSourceThumbnailMaxPixelSize: max(pixels, 1),
        ] as CFDictionary
        guard let image = CGImageSourceCreateThumbnailAtIndex(source, 0, small) else { return nil }
        cache.setObject(image, forKey: key)
        return image
    }
}

/// What a macro is, in the square beside its name: its first attachment, or
/// a glyph for text.
struct MacroBadge: View {
    let macro: Macro
    let shelf: Shelf?
    var side: CGFloat = 44

    @Environment(\.colorScheme) private var scheme

    var body: some View {
        if let first = macro.attachments.first {
            Thumbnail(
                attachment: first,
                url: shelf.flatMap { $0.has(first) ? $0.url(for: first) : nil },
                side: side
            )
        } else {
            ZStack {
                RoundedRectangle(cornerRadius: side * 0.22, style: .continuous)
                    .fill(Theme.raised(scheme))
                Image(systemName: "text.alignleft")
                    .font(.system(size: side * 0.38))
                    .foregroundStyle(.secondary)
            }
            .frame(width: side, height: side)
        }
    }
}

extension Macro {
    /// One line saying what is in it, for under its name.
    var summary: String {
        let text = body
            .split(whereSeparator: \.isNewline)
            .first
            .map { $0.trimmingCharacters(in: .whitespaces) } ?? ""
        if !description.isEmpty { return description }
        if !text.isEmpty { return text }
        switch attachments.count {
        case 0: return "Empty"
        case 1: return "\(attachments[0].name) · \(formatBytes(attachments[0].bytes))"
        default: return "\(attachments.count) attachments · \(formatBytes(attachments.reduce(0) { $0 + $1.bytes }))"
        }
    }
}
