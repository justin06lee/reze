//! The pasteboard, for everything that is not plain text.
//!
//! `arboard` covers text and nothing else here needs, so it stays in
//! [`crate::paste`]. Files are a different matter: there is no cross-platform
//! notion of "the clipboard holds these three files", and on macOS it is an
//! `NSPasteboard` holding one item per file carrying `public.file-url`. That is
//! what Finder writes on ⌘C and what every app that accepts a pasted file reads,
//! so it is what we write too.

use std::path::{Path, PathBuf};

/// Something the user copied that could become an attachment.
pub enum Incoming {
    /// A file that already exists on disk. Preferred when offered: it has a
    /// real name and exact bytes, where re-encoded pasteboard data has neither.
    File(PathBuf),
    /// Bytes with no file behind them — a screenshot, an image dragged out of a
    /// browser.
    Data { mime: String, bytes: Vec<u8> },
}

/// Image types worth putting on the pasteboard *alongside* the file, so an app
/// that wants a picture inline gets one without reading the file itself.
///
/// The pasteboard type is a UTI. Only PNG and TIFF have AppKit constants; the
/// other two are spelled out because the constant does not exist, not because
/// the identifier is uncertain.
const INLINE_IMAGE_UTIS: [(&str, &str); 4] = [
    ("image/png", "public.png"),
    ("image/jpeg", "public.jpeg"),
    ("image/gif", "com.compuserve.gif"),
    ("image/tiff", "public.tiff"),
];

/// Never read more than this off the pasteboard in one go. A pasteboard can
/// legitimately hold a whole Finder selection; silently importing four hundred
/// files because someone pressed ⌘A first is not what anyone meant.
const MAX_INCOMING: usize = 24;

#[cfg(target_os = "macos")]
mod imp {
    use super::*;
    use objc2::runtime::ProtocolObject;
    use objc2_app_kit::{
        NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypePNG,
        NSPasteboardTypeTIFF,
    };
    use objc2_foundation::{NSArray, NSData, NSString, NSURL};

    /// Files and images sitting on the pasteboard right now.
    pub fn read_media() -> Vec<Incoming> {
        let pb = NSPasteboard::generalPasteboard();
        let mut out = Vec::new();

        // File URLs first and exclusively: an image copied in Finder is on the
        // pasteboard *both* as a file and as rendered data, and taking the file
        // keeps the original bytes and the original name.
        if let Some(items) = pb.pasteboardItems() {
            for item in items.iter() {
                if out.len() >= MAX_INCOMING {
                    break;
                }
                if let Some(path) = file_url_path(&item) {
                    out.push(Incoming::File(path));
                }
            }
        }
        if !out.is_empty() {
            return out;
        }

        // Nothing on disk, so take the rendered image. PNG before TIFF: both are
        // usually present, and TIFF is the lossless-but-enormous one macOS keeps
        // for compatibility.
        for (mime, ty) in [
            ("image/png", unsafe { NSPasteboardTypePNG }),
            ("image/tiff", unsafe { NSPasteboardTypeTIFF }),
        ] {
            if let Some(data) = pb.dataForType(ty) {
                let bytes = data.to_vec();
                if !bytes.is_empty() {
                    out.push(Incoming::Data {
                        mime: mime.to_string(),
                        bytes,
                    });
                    break;
                }
            }
        }
        out
    }

    fn file_url_path(item: &NSPasteboardItem) -> Option<PathBuf> {
        let raw = item.stringForType(unsafe { NSPasteboardTypeFileURL })?;
        // Percent-decoding via NSURL rather than by hand: the string is a URL,
        // and any file with a space or an accent in its name depends on it.
        let url = NSURL::URLWithString(&raw)?;
        Some(PathBuf::from(url.path()?.to_string()))
    }

    /// Replace the pasteboard contents with `files`.
    pub fn write_files(files: &[PathBuf]) -> Result<(), String> {
        let items: Vec<_> = files.iter().filter_map(|p| item_for(p)).collect();
        if items.is_empty() {
            return Err("none of those files could be put on the clipboard".into());
        }
        let objects: Vec<_> = items.into_iter().map(ProtocolObject::from_retained).collect();

        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        if pb.writeObjects(&NSArray::from_retained_slice(&objects)) {
            Ok(())
        } else {
            Err("the clipboard refused the files".into())
        }
    }

    fn item_for(path: &Path) -> Option<objc2::rc::Retained<NSPasteboardItem>> {
        let url = NSURL::fileURLWithPath(&NSString::from_str(path.to_str()?));
        let item = NSPasteboardItem::new();
        let absolute = url.absoluteString()?;
        item.setString_forType(&absolute, unsafe { NSPasteboardTypeFileURL });

        // Also hand over the image itself where an app might want it inline.
        // Read lazily and only for image types — a video would be pointless here
        // and expensive to load.
        if let Some(uti) = inline_uti(path) {
            if let Ok(bytes) = std::fs::read(path) {
                item.setData_forType(&NSData::with_bytes(&bytes), &NSString::from_str(uti));
            }
        }
        Some(item)
    }

    fn inline_uti(path: &Path) -> Option<&'static str> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        let mime = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "tif" | "tiff" => "image/tiff",
            _ => return None,
        };
        INLINE_IMAGE_UTIS
            .iter()
            .find(|(m, _)| *m == mime)
            .map(|(_, uti)| *uti)
    }
}

#[cfg(not(target_os = "macos"))]
mod imp {
    use super::*;

    /// No portable way to ask "what files are on the clipboard", and the X11
    /// answer is a different protocol again. The editor still takes drops and
    /// the file picker, which is where attachments actually come from.
    pub fn read_media() -> Vec<Incoming> {
        Vec::new()
    }

    pub fn write_files(_files: &[PathBuf]) -> Result<(), String> {
        Err("attachments can only be put on the clipboard on macOS".into())
    }
}

pub use imp::{read_media, write_files};
