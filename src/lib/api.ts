import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  Attachment,
  ClipEntry,
  Grabbed,
  GrabTools,
  Imported,
  ImportOutcome,
  ImportPreview,
  Library,
  LoginItemState,
  Tailnet,
} from "./types";

export const getLibrary = () => invoke<Library>("get_library");
export const saveLibrary = (library: Library) => invoke<void>("save_library", { library });
export const bumpUsage = (id: string) => invoke<void>("bump_usage", { id });

export const readClipboard = () => invoke<string>("read_clipboard");

export const accessibilityStatus = () => invoke<boolean>("accessibility_status");
export const requestAccessibility = () => invoke<boolean>("request_accessibility");
export const openAccessibilitySettings = () => invoke<void>("open_accessibility_settings");

export const loginItemStatus = () => invoke<LoginItemState>("login_item_status");
export const setLoginItem = (enabled: boolean) => invoke<void>("set_login_item", { enabled });
export const openLoginItemsSettings = () => invoke<void>("open_login_items_settings");

/**
 * Hide the palette and put `text` plus `attachments` into the app the user was
 * actually in. Whole attachments, since each knows for itself whether its bytes
 * are in the asset library or in a watched folder.
 */
export const deliver = (text: string, copyOnly: boolean, attachments: Attachment[]) =>
  invoke<void>("deliver", { text, copyOnly, attachments });

export const importFiles = (paths: string[]) => invoke<Imported>("import_files", { paths });
export const importClipboard = () => invoke<Imported>("import_clipboard");

export const hidePalette = () => invoke<void>("hide_palette");
/** Show without resetting — used when an in-place expansion needs values. */
export const showPalette = () => invoke<void>("show_palette");
export const openEditor = () => invoke<void>("open_editor");
/** Keep the hotkeys in step when the window changes mode by itself. */
export const setPaletteMode = (clipboard: boolean) =>
  invoke<void>("set_palette_mode", { clipboard });
/** Which mode was last asked for — see the command for why this is needed. */
export const paletteMode = () => invoke<boolean>("palette_mode");
export const quit = () => invoke<void>("quit");
export const resizePalette = (height: number) => invoke<void>("resize_palette", { height });
export const setHotkeys = (palette: string, expand: string, clipboard: string) =>
  invoke<void>("set_hotkeys", { palette, expand, clipboard });
export const libraryPath = () => invoke<string>("library_path");
export const revealLibrary = () => invoke<void>("reveal_library");
export const revealPath = (path: string) => invoke<void>("reveal_path", { path });

/** Replace the watched folder list and rescan. */
export const setFolders = (folders: string[]) => invoke<void>("set_folders", { folders });

export const grabTools = () => invoke<GrabTools>("grab_tools");

/**
 * Download `url` into the first watched folder. Resolves when it has finished,
 * which for a video is tens of seconds — the caller is expected to show that.
 */
export const grabUrl = (url: string) => invoke<Grabbed>("grab_url", { url });

/** Rename a just-grabbed file, which renames the macro made from it. */
export const renameGrab = (path: string, name: string) =>
  invoke<string>("rename_grab", { path, name });

// ------------------------------------------------------------ other devices

/**
 * Whether Tailscale is up, and which of your other machines run Reze.
 *
 * Slow by the standards of everything else here — it asks each machine and
 * waits a couple of seconds for the ones that do not answer.
 */
export const tailnetStatus = () => invoke<Tailnet>("tailnet_status");

/** What importing from the machine at `address` would do, without doing it. */
export const importPreview = (address: string) =>
  invoke<ImportPreview>("import_preview", { address });

/**
 * Bring that machine's library into this one. `replace` makes this library a
 * copy of it; otherwise what only this machine has is kept.
 */
export const importLibrary = (address: string, replace: boolean, settings: boolean) =>
  invoke<ImportOutcome>("import_library", { address, replace, settings });

/** Put the library back the way it was before the last import. */
export const undoImport = () => invoke<void>("undo_import");

// ------------------------------------------------------- clipboard history

/** Everything remembered, newest first. Previews only — see `ClipEntry`. */
export const clipboardHistory = () => invoke<ClipEntry[]>("clipboard_history");

/** Put a remembered entry back where the cursor is, or only on the clipboard. */
export const deliverClip = (id: string, copyOnly: boolean) =>
  invoke<void>("deliver_clip", { id, copyOnly });

export const forgetClip = (id: string) => invoke<void>("forget_clip", { id });
export const clearClips = () => invoke<void>("clear_clips");

/**
 * Where remembered images and source-app icons live.
 *
 * Cached the same way, and for the same reason, as `assetsDir` — every row in
 * the list wants it.
 */
let clipRoot: Promise<string> | null = null;
export const clipboardDir = () => {
  clipRoot ??= invoke<string>("clipboard_dir").catch((e) => {
    clipRoot = null;
    throw e;
  });
  return clipRoot;
};

/** A URL for a remembered image. */
export async function clipUrl(file: string): Promise<string> {
  return convertFileSrc(`${await clipboardDir()}/${file}`);
}

/** A URL for a source app's icon. */
export async function clipIconUrl(icon: string): Promise<string> {
  return convertFileSrc(`${await clipboardDir()}/icons/${icon}`);
}

/**
 * Where attachment files live on disk.
 *
 * Fetched once and cached: it never changes while the app runs, and every
 * thumbnail needs it. Resolved lazily rather than at module load so a failed
 * call retries on the next preview instead of leaving the editor imageless.
 */
let assetRoot: Promise<string> | null = null;
export const assetsDir = () => {
  assetRoot ??= invoke<string>("assets_dir").catch((e) => {
    assetRoot = null; // so the next preview tries again rather than staying blank
    throw e;
  });
  return assetRoot;
};

/** A URL the webview can load an attachment from — see the asset protocol. */
export async function attachmentUrl(attachment: Attachment): Promise<string> {
  // A watched folder's file stays where it is, so it is already a full path.
  const path = attachment.path ?? `${await assetsDir()}/${attachment.file}`;
  return convertFileSrc(path);
}
