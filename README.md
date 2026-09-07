<div align="center">

<img src="assets/reze.svg" alt="Reze" width="640" />

# Reze

**A menu-bar app that turns short triggers into full prompts — and the images and video that go with them.**

</div>

---

Built for macOS. It also builds and runs on Linux under X11, with the
macOS-specific pieces degrading rather than breaking: the palette paints its own
surface instead of using the native blur, the tray gets the colour icon instead
of a template glyph, and expansion uses the selection fallback because the
keystroke tap is macOS-only. **Wayland is not supported** — it blocks synthetic
input into other applications by design, which is the mechanism this whole app
rests on.

Hit a global hotkey anywhere, type `full analysis`, press Enter — the paragraph
you actually meant gets pasted into whatever app you were already in.

```
┌──────────────────────────────────────────────────────────────┐
│ 🔍  full an                                              3   │
├──────────────────────┬───────────────────────────────────────┤
│ ▸ full analysis      │ Deep end-to-end read of a codebase    │
│   #review #analysis  │ ───────────────────────────────────── │
│   security audit     │ Be rigorous and concrete. Cite every  │
│   #review #security  │ claim with a `file:line` reference…   │
└──────────────────────┴───────────────────────────────────────┘
  ↵ select   ⌘↵ copy only   esc close          Edit macros
```

## Install

```bash
make
```

One command is the whole path: it stops any running copy, clears the stale
Accessibility entry left by a previous install, builds the app with bun, puts a
fresh **Reze.app** in `/Applications`, and launches it. After pulling changes,
`make update` runs the same refresh.

`make build` alone produces both bundles under
`src-tauri/target/release/bundle/` without touching `/Applications`:

| Path | What |
| --- | --- |
| `macos/Reze.app` | The app itself (~12 MB) |
| `dmg/Reze_<version>_aarch64.dmg` | Disk image (~4 MB) with a drag-to-`Applications` layout |

There is no Dock icon and no window on launch — it is a menu-bar app, so look
for the bomb glyph up top. Press `⌘⇧Space` to confirm it is alive.

Two things to know on first run:

- **Grant Accessibility when macOS asks.** The permission is tied to the exact
  binary, so a dev build, the installed app, and every rebuilt version of it
  are all separate entries — and a stale one can keep looking ticked in System
  Settings while being inert. `make` and `make update` reset the stale entry
  before installing, so the fresh app prompts cleanly; all that is left is to
  allow it. Until then the palette still works, and `⌘↵` still copies to the
  clipboard.
- **It is ad-hoc signed, not notarized.** Building and running it on your own
  machine is fine — locally built apps never get the quarantine flag. If you
  copy the `.dmg` to another Mac it *will* be flagged, and Gatekeeper will
  refuse to open it. There, either right-click the app → **Open**, or strip the
  flag: `xattr -dr com.apple.quarantine /Applications/Reze.app`. Signing it
  properly needs a paid Apple Developer ID.

`make build` targets the host architecture — `aarch64` on Apple Silicon. For a
binary that also runs on Intel Macs, build
`bun run tauri build --target universal-apple-darwin` (requires
`rustup target add x86_64-apple-darwin`).

To have it running from the moment you log in, turn on **Start Reze when I log
in** under Settings in the editor. That registers the app with macOS itself, so
it also appears under System Settings → General → Login Items — the two are the
same switch, and either one can turn it off. Needs macOS 13 or later, and the
installed `Reze.app` rather than a dev build.

Reinstalling replaces the app bundle and clears the registration along with the
Accessibility grant, so tick it again after `make update`.

## Using it

There are two ways in. **Search** it when you want to browse, or **expand in
place** when you already know the trigger.

### Expand in place

Type a trigger straight into whatever you are already writing in, then press
`⌥Space`:

```
Please do a full analysis|          ← your caret, in any app
                    ⌥Space
Please do a Be rigorous and concrete. Cite every claim with…
```

Reze replaces the trigger with the expansion. No window appears, and the rest of
the line is left exactly as it was. If the words match nothing, nothing happens
and your text is untouched.

It works this out in one of two ways:

1. **What it saw you type.** Reze keeps the last 160 characters you typed and
   backspaces over the trigger. This is the only approach that works in a
   terminal or any TUI — `Option+←` there is an escape sequence to the shell,
   not a selection, and `⌘C` copies the terminal's view selection rather than
   the line you are editing. See [Typing awareness](#typing-awareness).
2. **Selecting and reading back the words at your caret.** Used when the first
   has nothing to offer — text typed before Reze started, or pasted rather than
   typed. Works in ordinary text fields, not in terminals.

Matching ignores case and prefers the longest trigger, so with both `analysis`
and `full analysis` defined, typing "full analysis" expands the longer one. If
the macro has variables, the palette opens on its fill-in step and the
replacement happens once you submit.

This needs a real key combination — a bare `Tab` cannot be used, because a
global shortcut swallows that key in *every* application.

### The palette

| Key | Does |
| --- | --- |
| `⌘⇧Space` | Open the palette over the current app (configurable) |
| `⌘⇧V` | Open it on the [clipboard history](#clipboard-history) instead |
| `↑` `↓` / `⌃N` `⌃P` | Move through results |
| `↵` | Expand and paste — or open the fill-in step if the macro has variables |
| `⌘↵` | Put the expansion on the clipboard instead of pasting |
| `esc` | Back out of fill-in, or close |

Paste a **link** into the search field instead of a search and it turns into a
download. See [Pasting a link](#pasting-a-link).

The tray icon opens the macro editor, where you can add, edit, tag, duplicate
and delete macros, [attach files](#attachments) to them, rebind the hotkeys,
change how expansions are delivered, and set Reze to start at login.

### Quitting

Use **Quit Reze** in the tray menu, or `⌘Q` with the editor focused. A menu-bar
app has no application menu, so `⌘Q` has nothing to hang off by default — the
editor window handles it itself. `⌘W` there closes the window without quitting.

## Attachments

A macro can carry files as well as text — a screenshot, a reference video, a
PDF, anything. Trigger it and they arrive in whatever app you were in, the same
way the text does.

Three ways to attach one, in the editor:

- **Drop it** anywhere on the window.
- **`⌘V`** with a file or an image copied. Reze reads the real pasteboard rather
  than what the web view saw, so a file copied in Finder works as well as a
  screenshot taken with `⌘⇧4`.
- **Add file…** for the ordinary file picker.

Dropping onto an empty editor with nothing selected makes a new macro named
after the file, rather than refusing the drop.

### How they are delivered

A clipboard holds one payload, so a macro carrying both takes two passes:
**the text first, then the files.** That leaves the text landing exactly where a
text-only macro would put it — surrounding characters and all — and the
attachments follow. In a chat composer they drop into the attachment tray; in a
document they land after the paragraph.

`⌘↵` copies instead of pasting, and can only copy one thing: the text if there
is any, otherwise the files.

Images go on the clipboard as the file *and* as the picture, so an app that
inlines images gets one without reading the file, and an app that wants a file
gets a real one. Everything else goes as a file.

This part is macOS-only — there is no portable notion of "the clipboard holds
these three files", and the X11 answer is a different protocol again. Elsewhere
the text is still delivered and the attachments are reported as skipped.

### Where they live

Alongside the library, in `~/.reze/assets/`, under a generated name that
`macros.json` refers to:

```jsonc
"attachments": [
  {
    "id": "…",
    "file": "a1b2c3d4-….png",   // the name inside ~/.reze/assets
    "name": "Screenshot.png",   // what it was called when it came in
    "kind": "image",            // image | video | audio | file
    "mime": "image/png",
    "bytes": 261314
  }
]
```

Files are **copied in**, not linked, so a macro keeps working after the
screenshot it was made from is thrown away and the whole `~/.reze` directory
stays portable between machines.

Files no macro refers to any more are deleted after a save in the app — never
after an edit you make to `macros.json` yourself, because a text editor
mid-write can momentarily look like a library with no macros in it, and
sweeping on that would take every attachment with it. Anything imported in the
last minute is spared regardless, since a dropped file exists for a moment
before the macro that references it is written.

### Watched folders

Attaching files one at a time is fine for a handful. For a collection, point
Reze at the folder instead:

**Settings → Folders → Add folder…**

Every file in it becomes a macro on its own, named after the file, with its
subfolders as tags. Add a file and the macro appears; delete it and the macro
goes; rename it and the trigger changes. A folder like

```
~/refs/
├── memes/
│   └── cats/
│       └── tabby.png     →  trigger "tabby",  tags #memes #cats
└── logo.svg              →  trigger "logo"
```

is a library of two macros without a word typed into the editor.

These are **derived**, not authored, so:

- They are never written into `macros.json`. The folder is what they are made
  of, the way that file is for the ones you wrote.
- Their files are read where they lie — nothing is copied, because that folder
  is already yours and shadowing a hundred gigabytes of video inside `~/.reze`
  would be absurd.
- The editor shows them read-only. To make one yours — to add text around it,
  say — press **Make an editable copy**, which copies its file into the asset
  library and hands you an ordinary macro.
- Where a trigger name is claimed twice, the one you wrote wins.

Dotfiles are skipped, subfolders are followed eight deep, and the scan stops at
two thousand files so that pointing this at a home directory degrades rather
than hangs.

## Pasting a link

Open the palette and paste the URL of a post where you would type a search.
Reze recognises that it is a link rather than a query and offers to download it
instead:

```
┌──────────────────────────────────────────────────────────────┐
│ 🔍  https://www.youtube.com/watch?v=…                        │
├──────────────────────────────────────────────────────────────┤
│ Download from youtube.com                                    │
│ https://www.youtube.com/watch?v=…                            │
└──────────────────────────────────────────────────────────────┘
  ↵ download
```

`↵` downloads it into the first watched folder. When it lands, the palette asks
what to call it, with the name it worked out already filled in and selected —
so `↵` again accepts it, or type over it first:

```
┌──────────────────────────────────────────────────────────────┐
│ ↓  Downloaded                                                │
├──────────────────────────┬───────────────────────────────────┤
│ CALL IT                  │                                   │
│ ┌──────────────────────┐ │         ▶                         │
│ │ youtube - jawed - Me…│ │                                   │
│ └──────────────────────┘ │                                   │
└──────────────────────────┴───────────────────────────────────┘
  ↵ save   esc keep this name
```

The name you give it renames the file, and the file is what the macro is named
after — so it is a macro called exactly that, immediately.

Names are worked out from the post: the site, who posted it, and the title.
That is the point — a folder of `video.mp4` and `image.jpg` would be useless a
week later.

`esc` during a download leaves the naming step, not the download: it finishes
in the background and keeps the name it was given.

### What it uses

Nothing is bundled. Three tools are looked for, and **Settings → Downloading a
link** says which are present and what to type for the rest:

| Tool | For | Without it |
| --- | --- | --- |
| `yt-dlp` | Video, and the site knowledge for most of the web | No video from any post |
| `gallery-dl` | Images, which `yt-dlp` deliberately does not do | Image-only posts fail |
| `ffmpeg` | Joining the separate streams YouTube serves | Video capped around 720p |

```bash
brew install yt-dlp gallery-dl ffmpeg
```

They are searched for on `PATH` and in the usual Homebrew and MacPorts
locations, because an app launched from Finder inherits a `PATH` from `launchd`
that has none of them in it.

A link straight to a file — `…/photo.png` — skips all three and is fetched with
`curl`, so that much works with nothing installed.

Video is asked for as **H.264 in mp4** where the site offers it. Asking for the
container alone is not enough: AV1 is also mp4, and macOS plays almost none of
it — not in Messages, not in Keynote, not in this app's own preview.

### What it will not do

Only public posts. Nothing here logs in, supplies credentials, or works around
access control — anything you could not open in a browser cannot be grabbed
either. It is a faster way to save what you can already see, and respecting
whatever the site's terms allow you to save is on you.

## Clipboard history

Press `⌘⇧V` anywhere and the same palette opens on everything you have copied,
newest first — **with the app you copied it from**.

```
┌──────────────────────────────────────────────────────────────┐
│ 📋  Search clipboard…                                        │
├───────────────────────────────┬──────────────────────────────┤
│ ▸ ▤ˢ update the version, com… │ Safari              4:31 PM  │
│     Text · Safari · Copied 4… │ ──────────────────────────── │
│   ▤ᵗ SELECT * FROM macros WH… │ update the version, commit,  │
│     Text · Terminal · Copied… │ push, open a pr, and merge…  │
│   ▨ᶜ                          │                              │
│     PNG image · Chrome · Cop… │                              │
└───────────────────────────────┴──────────────────────────────┘
  ↵ paste   ⌘↵ copy only   ⌘⌫ forget   esc close       Macros
```

| Key | Does |
| --- | --- |
| `↑` `↓` / `⌃N` `⌃P` | Move through the list |
| `↵` | Paste it back into the app you were in |
| `⌘↵` | Put it on the clipboard without pasting |
| `⌘⌫` | Forget this one |
| `esc` | Close |

Typing filters. Unlike the macro search this is a plain substring match, not a
fuzzy one — a clipboard entry is up to eight hundred characters of arbitrary
text, and a fuzzy match against that would find everything.

**Text, images and files all come back as what they were.** An image copied out
of a browser goes back onto the pasteboard as an image, not as a file attachment
named after a UUID. Files copied in Finder are remembered where they lie and
pasted as files. Text is pasted as text, including the part too long to show in
the preview.

### Where the app icon comes from

Nothing on the pasteboard says who wrote it — macOS records no owner. So Reze
takes whoever was frontmost at the moment the pasteboard changed, which is the
same inference every clipboard manager on the platform makes. It is right unless
you copy and switch apps inside the same half second. The icon is asked of the
running app once and cached in `~/.reze/clipboard/icons/`, so a hundred entries
from Safari cost one icon.

### What it does not record

- **Anything an app marks private.** Password managers and their kind put
  `org.nspasteboard.ConcealedType` (or `TransientType`, or `AutoGeneratedType`)
  on the pasteboard alongside the value. Reze honours all of them and skips the
  copy entirely.
- **Its own traffic.** Reze writes to the pasteboard constantly — every paste,
  every clipboard restore, and the synthesized `⌘C` that
  [expand in place](#expand-in-place) uses to read the words at your caret.
  Recording would fill the history with the app.
- **Anything copied before it started.** There is no backlog to read.
- **Text over 1 MB, or images over 32 MB.** The index is rewritten on every
  copy, and one pasted log file would make every copy after it pay for that.

### Where it lives

`~/.reze/clipboard.json`, with copied images in `~/.reze/clipboard/`. **In plain
text, on this machine, and nowhere else** — nothing is sent anywhere. It is
deliberately not part of `macros.json`: that file is watched for external edits
and rebroadcast to both windows, and a history rewriting it on every copy would
have them repainting all day.

Settings → **Clipboard history** rebinds the hotkey, sets how many items to keep
(200 by default), switches recording off, and clears the lot.

## Template syntax

Deliberately tiny — anything more expressive belongs in the prompt itself.

| Syntax | Meaning |
| --- | --- |
| `{{target}}` | A variable. The palette prompts you for it before pasting. |
| `{{target\|this codebase}}` | Same, pre-filled with a default. |
| `{{> rigor}}` | Inline another macro by its trigger, so shared preamble lives in one place. |
| `{{clipboard}}` | Whatever you last copied. |
| `{{date}}` `{{time}}` `{{datetime}}` | Filled automatically, never prompted. |

Includes nest, and a macro that (transitively) includes itself is reported in
the editor rather than recursing forever.

## Storage

Everything lives in `~/.reze/macros.json`, and **that file is the source of
truth** — the GUI is a convenience layer over it. Edit it in any text editor and
Reze picks up the change immediately; edit it in the app and your editor sees
the change on next read. Diff it, commit it, sync it however you like.

```jsonc
{
  "version": 1,
  "settings": {
    "hotkey": "CmdOrCtrl+Shift+Space",
    "expandHotkey": "Alt+Space", // expand the trigger at the caret
    "folders": ["~/.reze/media"],// every file in these becomes a macro
    "trackTyping": true,         // needed for expansion inside terminals
    "pasteMode": "paste",        // or "copy" to never paste directly
    "restoreClipboard": true,
    "clipboardHotkey": "CmdOrCtrl+Shift+V",
    "clipboardHistory": true,    // remember what you copy
    "clipboardLimit": 200
  },
  "macros": [
    {
      "id": "…",
      "name": "full analysis",
      "description": "Deep end-to-end read of a codebase",
      "tags": ["review"],
      "body": "{{> rigor}}\n\nFully and thoroughly analyze {{target|this codebase}}…",
      "attachments": [], // see Attachments
      "usageCount": 0
    }
  ]
}
```

The [clipboard history](#clipboard-history) is deliberately *not* in here — it
has its own file, `~/.reze/clipboard.json`, because this one is watched for
external edits and a history rewriting it on every copy would never stop.

Two settings are deliberately not in here: the Accessibility grant and the
launch-at-login switch. Both are owned by macOS, and a copy of the answer in
this file would start lying the moment you changed it in System Settings — so
Reze reads them back from the OS instead of remembering them.

## Typing awareness

Expanding in a terminal requires knowing what you typed, and nothing can be
read back out of a terminal's line editor. So Reze watches typing instead:

- It keeps the **last 160 characters**, in memory only. Never written to disk,
  never sent anywhere.
- It is discarded on Enter, Tab, Escape, any arrow or navigation key, any mouse
  click, any keypress with a modifier held, and whenever you switch apps.
- macOS disables event taps entirely while **secure input** is active, so
  password fields are never observed — not by policy, but by the OS.
- Turn it off with `trackTyping` in Settings. Expansion then falls back to
  selecting the words at your caret, which works in normal text fields but not
  in a terminal.

Only implemented on macOS. Elsewhere the selection fallback is always used.

The tap needs Accessibility permission to exist at all, so granting it *after*
launch leaves it uninstalled. Reze retries on the next expansion rather than
making you restart — that one attempt still falls back to selection.

## Accessibility permission

macOS has no supported way to inject text into another app, so pasting works by
putting the expansion on the clipboard and synthesizing `⌘V`. Expanding in place
additionally selects and copies the words before your caret to find out what you
typed. Both need **Accessibility** permission, and without it the keystrokes are
silently swallowed — the palette opens, and nothing arrives.

The editor shows a banner with a button that jumps straight to
System Settings → Privacy & Security → Accessibility when the permission is
missing. Your previous clipboard contents are restored a moment after the paste
lands (turn that off in Settings if you'd rather they weren't).

Note that the permission is granted per-binary: the dev build and a bundled
`Reze.app` are separate entries, and each needs granting once.

## Development

```bash
bun install
bun run tauri dev      # run it
bun run tauri build    # produce Reze.app + a .dmg in src-tauri/target/release/bundle
```

- `assets/icon.svg`, `assets/tray.svg` — icon sources (see below)
- `src-tauri/src/store.rs` — the JSON library, its schema, and the seed macros
- `src-tauri/src/paste.rs` — clipboard + synthesized keystrokes, and the permission check
- `src-tauri/src/assets.rs` — the attachment library on disk (`cargo test`)
- `src-tauri/src/clip.rs` — the pasteboard, for everything that is not text
- `src-tauri/src/folders.rs` — watched folders becoming macros (`cargo test`)
- `src-tauri/src/grab.rs` — a URL becoming a file (`cargo test`)
- `src-tauri/src/expand.rs` — which trigger the typed words name (`cargo test`)
- `src-tauri/src/typed.rs` — the keystroke tap behind terminal support
- `src-tauri/src/focus.rs` — remembering who had focus, and pasteboard change detection
- `src-tauri/src/login.rs` — the launch-at-login switch, via `SMAppService`
- `src-tauri/src/lib.rs` — windows, tray, global hotkeys, file watcher, IPC commands
- `src/lib/template.ts` — the template language (variables, includes, built-ins)
- `src/lib/fuzzy.ts` — the palette's ranking
- `src/Attachments.tsx` — attachment previews, shared by both windows
- `src/Palette.tsx`, `src/Editor.tsx` — the two windows, routed by window label

Both windows load the same bundle; `main.tsx` picks a component from
`getCurrentWindow().label`.

## Icons

Original artwork, hand-written SVG, in `assets/`. Both are a bomb-head wearing a
pulled grenade pin — a nod to the Bomb Devil's pin-in-the-neck that doubles as
the app's premise: pull a tiny pin, get an explosion.

| Source | Becomes | Notes |
| --- | --- | --- |
| `assets/icon.svg` | `src-tauri/icons/*` incl. `icon.icns`, `icon.ico` | Authored at 1024² with the macOS icon grid baked in (824² body, 100px margin), so generation only ever downscales. |
| `assets/tray.svg` | `src-tauri/icons/tray.png` | The menu-bar glyph. |

Regenerate both after editing either SVG (needs `rsvg-convert` — `brew install librsvg`):

```bash
bun run icons
```

The tray glyph is a macOS **template image**: pure black plus alpha, nothing
else. The system ignores the colour and tints the alpha itself, so it comes out
white on a dark menu bar and black on a light one automatically — drawing it
white would break that. `tray-icon` normalises menu-bar images to 18pt tall
preserving aspect ratio, so the glyph is authored square at 72px (4× for
Retina) with deliberately fat strokes that survive the downscale. It is
embedded with `include_bytes!` rather than bundled as a resource, so it cannot
go missing at runtime.

## Building

Always build with `bun run tauri build` (what `make build` runs) and install
the whole `.app`.

Running `cargo build` and copying just the binary over an installed bundle
looks like it works and does not: the frontend is embedded at compile time, and
a partially-rebuilt binary can end up with an `index.html` that references an
asset hash the embedded map no longer has. The window then loads, `#root` stays
empty, and no error surfaces anywhere — the app simply does nothing. If you ever
see that, `cargo clean -p reze` and build again properly.

## License

MIT — see [LICENSE](LICENSE).
