//! Turning a URL into a file on disk.
//!
//! Copy a link to a post, press the grab hotkey, and whatever the post is made
//! of lands in your first watched folder — which means it is a macro a moment
//! later, without touching the editor.
//!
//! The actual downloading is delegated to `yt-dlp` and `gallery-dl`, the two
//! tools that already know how a thousand sites lay out their media and that
//! keep up when those sites change. Reimplementing that here would be a
//! full-time job done worse. Neither is bundled: they are found on `PATH` if
//! present, and their absence is reported with the command to install them.
//!
//! Only public media is in scope. Nothing here logs in, supplies credentials or
//! works around access control — a post you cannot open in a browser is a post
//! this cannot fetch either.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// How long any one download may take before it is killed.
///
/// Generous: a long video over a bad connection is normal. The point is only
/// that a wedged process cannot hold the grab lock forever.
const TIMEOUT: Duration = Duration::from_secs(15 * 60);

/// Directories to search besides `PATH`.
///
/// An app launched from Finder inherits a minimal `PATH` from `launchd` —
/// typically without `/opt/homebrew/bin` — so a tool the user installed and can
/// run in their terminal is invisible to us unless we go looking.
const EXTRA_BIN_DIRS: [&str; 5] = [
    "/opt/homebrew/bin",
    "/usr/local/bin",
    "/opt/local/bin", // MacPorts
    "~/.local/bin",
    "~/bin",
];

/// How the tools are installed here, for the messages that name them. On
/// Linux not the distribution's packages: those lag months behind the sites,
/// and a yt-dlp a few months old no longer downloads from YouTube at all.
#[cfg(target_os = "macos")]
const INSTALL: &str = "brew install";
#[cfg(not(target_os = "macos"))]
const INSTALL: &str = "pipx install";

/// Which download tools are actually present.
#[derive(Default, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Tools {
    /// Video, and the site knowledge for most of the web.
    pub yt_dlp: Option<String>,
    /// Images, which `yt-dlp` deliberately does not do.
    pub gallery_dl: Option<String>,
    /// Not a downloader — `yt-dlp` needs it to join the separate video and
    /// audio streams YouTube serves for anything above 720p.
    pub ffmpeg: Option<String>,
}

pub fn tools() -> Tools {
    Tools {
        yt_dlp: which("yt-dlp"),
        gallery_dl: which("gallery-dl"),
        ffmpeg: which("ffmpeg"),
    }
}

/// Find a tool by name. Also how [`crate::tailnet`] finds the Tailscale CLI,
/// which is hidden from a launched app for exactly the same reason.
pub fn which(name: &str) -> Option<String> {
    // `PATH` first, so a version the user put ahead of Homebrew's still wins.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join(name);
            if is_executable(&candidate) {
                return Some(candidate.to_string_lossy().to_string());
            }
        }
    }
    for dir in EXTRA_BIN_DIRS {
        let candidate = PathBuf::from(expand_home(dir)).join(name);
        if is_executable(&candidate) {
            return Some(candidate.to_string_lossy().to_string());
        }
    }
    None
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(path)
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

fn expand_home(path: &str) -> String {
    match (path.strip_prefix('~'), dirs::home_dir()) {
        (Some(rest), Some(home)) => format!("{}{}", home.display(), rest),
        _ => path.to_string(),
    }
}

/// What a grab produced.
pub struct Grabbed {
    pub path: PathBuf,
}

/// Download whatever `url` points at into `dir`.
pub fn grab(url: &str, dir: &Path) -> Result<Grabbed, String> {
    let url = url.trim();
    // Only the two schemes a post can live at. `file://` would turn the grab
    // hotkey into "copy an arbitrary path off this machine", and a clipboard is
    // not a trustworthy place to take a path from.
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err("That is not a link — copy the URL of a post first.".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {dir:?}: {e}"))?;

    // A link straight to a file needs no extractor and no installed tool.
    if let Some(name) = direct_media_name(url) {
        return download_direct(url, dir, &name);
    }

    let tools = tools();
    let mut problems = Vec::new();

    // Video first: it is what most links are, and yt-dlp knows the most sites.
    if let Some(bin) = &tools.yt_dlp {
        match run_yt_dlp(bin, tools.ffmpeg.as_deref(), url, dir) {
            Ok(path) => return Ok(Grabbed { path }),
            Err(e) => problems.push(e),
        }
    }

    // yt-dlp reports "no video in this post" for an image-only one, which is
    // exactly when gallery-dl is the right tool. Falling back on any failure
    // rather than on the wording of that message, which is not an API.
    if let Some(bin) = &tools.gallery_dl {
        match run_gallery_dl(bin, url, dir) {
            Ok(path) => return Ok(Grabbed { path }),
            Err(e) => problems.push(e),
        }
    }

    if tools.yt_dlp.is_none() && tools.gallery_dl.is_none() {
        return Err(
            format!(
                "Grabbing needs yt-dlp for video and gallery-dl for images. \
                 Install them with `{INSTALL} yt-dlp gallery-dl`."
            ),
        );
    }
    if tools.gallery_dl.is_none() {
        problems.push(
            format!(
                "No image extractor — install one with `{INSTALL} gallery-dl` if that post \
                 is images rather than video."
            ),
        );
    }
    Err(problems.join("\n\n"))
}

/// The filename to save a direct link to a media file under, or `None` if the
/// URL does not obviously point at one.
fn direct_media_name(url: &str) -> Option<String> {
    // Everything after the last `/`, minus any query string or fragment.
    let without_scheme = url.split_once("://")?.1;
    let (host, path) = without_scheme.split_once('/')?;
    let last = path
        .split(['?', '#'])
        .next()?
        .rsplit('/')
        .next()
        .filter(|s| !s.is_empty())?;

    let ext = last.rsplit_once('.')?.1.to_lowercase();
    const DIRECT: [&str; 20] = [
        "png", "jpg", "jpeg", "gif", "webp", "avif", "heic", "bmp", "tif", "tiff", "svg", "mp4",
        "mov", "webm", "mkv", "m4v", "mp3", "m4a", "wav", "pdf",
    ];
    if !DIRECT.contains(&ext.as_str()) {
        return None;
    }
    // Prefixed with the host so a folder of `image.jpg` files is still readable.
    Some(sanitize(&format!("{host} - {}", percent_decode(last))))
}

fn download_direct(url: &str, dir: &Path, name: &str) -> Result<Grabbed, String> {
    let dest = unique(dir, name);
    // curl rather than an HTTP crate: it ships with macOS, handles redirects and
    // TLS, and this is already a module that shells out.
    let status = run(
        Command::new("curl")
            .arg("--location")
            .arg("--fail")
            .arg("--silent")
            .arg("--show-error")
            .arg("--output")
            .arg(&dest)
            .arg("--")
            .arg(url),
        "curl",
    )?;
    if !status.0 {
        let _ = std::fs::remove_file(&dest);
        return Err(format!("Could not download that file.\n\n{}", status.1));
    }
    Ok(Grabbed { path: dest })
}

fn run_yt_dlp(bin: &str, ffmpeg: Option<&str>, url: &str, dir: &Path) -> Result<PathBuf, String> {
    let mut cmd = Command::new(bin);
    cmd.arg("--no-playlist") // a link with `&list=` means that video, not the playlist
        .arg("--no-progress")
        .arg("--no-warnings")
        // Prefer H.264 in mp4 over whatever is technically best. The point of a
        // grab is to paste the result into something else, and macOS decodes
        // H.264 everywhere while webm is refused outright and AV1 — which is
        // *also* mp4, so asking for the container alone is not enough — plays
        // in almost nothing, including the preview in this app. Each
        // alternative after the first loosens one constraint, so a site that
        // offers none of it still yields a file.
        .arg("--format")
        .arg(
            "bv*[vcodec^=avc1][ext=mp4]+ba[ext=m4a]/b[vcodec^=avc1][ext=mp4]\
             /bv*[ext=mp4]+ba[ext=m4a]/b[ext=mp4]/bv*+ba/b",
        )
        .arg("--merge-output-format")
        .arg("mp4")
        // Trims the whole name rather than one field, so the extension survives.
        .arg("--trim-filenames")
        .arg("120")
        .arg("--paths")
        .arg(dir)
        .arg("-o")
        .arg("%(extractor)s - %(uploader,channel,creator|unknown)s - %(title)s.%(ext)s")
        // The exact final path, after any merge or remux moved it.
        .arg("--print")
        .arg("after_move:filepath")
        .arg("--no-simulate");

    // Without ffmpeg, yt-dlp silently falls back to whatever single stream is
    // already muxed — usually 720p. Point it at ours if we found one.
    if let Some(ffmpeg) = ffmpeg {
        cmd.arg("--ffmpeg-location").arg(ffmpeg);
    }

    // `--` so a URL beginning with a dash cannot be read as a flag.
    cmd.arg("--").arg(url);

    let (ok, output) = run(&mut cmd, "yt-dlp")?;
    if !ok {
        return Err(format!("yt-dlp could not fetch that link.\n\n{output}"));
    }
    // The printed path is on stdout; anything else it said is on stderr.
    output
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| "yt-dlp finished but produced no file.".to_string())
}

fn run_gallery_dl(bin: &str, url: &str, dir: &Path) -> Result<PathBuf, String> {
    // gallery-dl has no equivalent of `--print filepath`, so the directory is
    // compared before and after. Grabs are serialised by the caller, so nothing
    // else can be writing here at the same time.
    let before = snapshot(dir);

    let (ok, output) = run(
        Command::new(bin)
            .arg("--dest")
            .arg(dir)
            // Flatten: the default nests by site and account, which would hide
            // the file from a non-recursive reading of the folder.
            .arg("--directory")
            .arg("")
            .arg("--quiet")
            .arg("--")
            .arg(url),
        "gallery-dl",
    )?;

    let mut fresh: Vec<PathBuf> = snapshot(dir)
        .into_iter()
        .filter(|p| !before.contains(p))
        .collect();

    if fresh.is_empty() {
        return Err(if ok {
            "gallery-dl found nothing to download at that link.".to_string()
        } else {
            format!("gallery-dl could not fetch that link.\n\n{output}")
        });
    }
    // A post with several images downloads all of them; report the first, and
    // the folder scan picks up the rest by itself.
    fresh.sort();
    Ok(fresh.remove(0))
}

fn snapshot(dir: &Path) -> std::collections::HashSet<PathBuf> {
    std::fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.path()).collect())
        .unwrap_or_default()
}

/// Run `cmd` to completion, killing it if it outlives [`TIMEOUT`].
///
/// Returns whether it succeeded and everything it said, both streams together —
/// these tools put their diagnosis on stderr and we want to show it verbatim.
fn run(cmd: &mut Command, name: &str) -> Result<(bool, String), String> {
    let mut child = cmd
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null())
        .spawn()
        .map_err(|e| format!("could not start {name}: {e}"))?;

    let started = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if started.elapsed() > TIMEOUT {
                    let _ = child.kill();
                    return Err(format!("{name} took too long and was stopped."));
                }
                std::thread::sleep(Duration::from_millis(120));
            }
            Err(e) => return Err(format!("waiting for {name}: {e}")),
        }
    }

    let out = child
        .wait_with_output()
        .map_err(|e| format!("reading {name} output: {e}"))?;
    let mut text = String::from_utf8_lossy(&out.stdout).to_string();
    let errors = String::from_utf8_lossy(&out.stderr);
    if !errors.trim().is_empty() {
        text.push('\n');
        text.push_str(&errors);
    }
    Ok((out.status.success(), text.trim().to_string()))
}

/// A path in `dir` for `name` that is not already taken.
fn unique(dir: &Path, name: &str) -> PathBuf {
    match name.rsplit_once('.') {
        Some((stem, ext)) => unique_named(dir, stem, &format!(".{ext}")),
        None => unique_named(dir, name, ""),
    }
}

/// A free path in `dir` for `stem` + `extension`, sanitizing the stem first.
///
/// Public because renaming a grab lands here too: the name then comes from a
/// text field rather than a URL, which is if anything more reason to run it
/// through [`sanitize`].
pub fn unique_named(dir: &Path, stem: &str, extension: &str) -> PathBuf {
    let stem = sanitize(stem);
    let candidate = dir.join(format!("{stem}{extension}"));
    if !candidate.exists() {
        return candidate;
    }
    for n in 2..1000 {
        let candidate = dir.join(format!("{stem} ({n}){extension}"));
        if !candidate.exists() {
            return candidate;
        }
    }
    dir.join(format!("{stem} ({}){extension}", uuid::Uuid::new_v4()))
}

/// Strip what a filename cannot contain. Deliberately narrow — the whole point
/// of these names is that they read like the post they came from.
fn sanitize(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '\0' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_start_matches('.').trim();
    if trimmed.is_empty() {
        "download".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

/// Just enough percent-decoding for a filename out of a URL path.
fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(byte) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_a_link_straight_to_a_file() {
        assert_eq!(
            direct_media_name("https://pbs.twimg.com/media/abc.jpg"),
            Some("pbs.twimg.com - abc.jpg".into()),
        );
        // Query strings are routine on media CDNs and are not part of the name.
        assert_eq!(
            direct_media_name("https://example.com/a/b/cat.png?width=900"),
            Some("example.com - cat.png".into()),
        );
    }

    #[test]
    fn leaves_post_urls_to_the_extractors() {
        for url in [
            "https://x.com/someone/status/123",
            "https://www.youtube.com/watch?v=abc",
            "https://example.com/",
            "https://example.com/page.html",
        ] {
            assert_eq!(direct_media_name(url), None, "{url} was taken for a file");
        }
    }

    #[test]
    fn decodes_escapes_in_the_filename() {
        assert_eq!(
            direct_media_name("https://example.com/my%20cat%20photo.png"),
            Some("example.com - my cat photo.png".into()),
        );
    }

    #[test]
    fn refuses_anything_that_is_not_a_web_link() {
        let dir = std::env::temp_dir();
        for url in ["file:///etc/passwd", "/etc/passwd", "ftp://x/y", ""] {
            assert!(grab(url, &dir).is_err(), "{url:?} was accepted");
        }
    }

    #[test]
    fn sanitizing_leaves_an_ordinary_name_alone() {
        // The point of these names is that they read like the post they came
        // from, so sanitizing must not mangle one that was already fine.
        let name = "youtube - Blender - Big Buck Bunny.mp4";
        assert_eq!(sanitize(name), name);
    }

    #[test]
    fn sanitizing_produces_something_that_cannot_escape_its_folder() {
        // What matters is not how it reads but that it stays a single path
        // component: no separator to climb with, no leading dot to hide behind.
        for hostile in ["../../etc/passwd", "a/b/c", "..", ".hidden", "x\0y", "  "] {
            let safe = sanitize(hostile);
            assert!(!safe.contains('/'), "{hostile:?} kept a separator: {safe:?}");
            assert!(!safe.contains('\\'), "{hostile:?} kept a separator: {safe:?}");
            assert!(!safe.starts_with('.'), "{hostile:?} stayed hidden: {safe:?}");
            assert!(!safe.is_empty(), "{hostile:?} sanitized to nothing");

            // The real guarantee: joining it cannot leave the folder.
            let dir = Path::new("/tmp/reze");
            assert_eq!(dir.join(&safe).parent(), Some(dir), "{hostile:?} escaped");
        }
    }

    #[test]
    fn renaming_sanitizes_and_keeps_the_extension() {
        // The rename path takes its name from a text field rather than a URL,
        // so this is the boundary that stops a typed "../" from moving a file
        // out of the watched folder.
        let dir = std::env::temp_dir().join(format!("reze-rename-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let plain = unique_named(&dir, "my clip", ".mp4");
        assert_eq!(plain, dir.join("my clip.mp4"));

        let hostile = unique_named(&dir, "../../escape", ".mp4");
        assert_eq!(hostile.parent(), Some(dir.as_path()), "a rename left the folder");
        assert!(hostile.extension().is_some(), "the extension was lost");

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn renaming_onto_a_taken_name_does_not_clobber_it() {
        let dir = std::env::temp_dir().join(format!("reze-rename2-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("taken.mp4"), b"keep me").unwrap();

        let next = unique_named(&dir, "taken", ".mp4");

        assert_eq!(next, dir.join("taken (2).mp4"));
        assert_eq!(std::fs::read(dir.join("taken.mp4")).unwrap(), b"keep me");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn does_not_overwrite_an_existing_file() {
        let dir = std::env::temp_dir().join(format!("reze-unique-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("cat.png"), b"x").unwrap();

        assert_eq!(unique(&dir, "cat.png"), dir.join("cat (2).png"));
        std::fs::remove_dir_all(&dir).ok();
    }
}
