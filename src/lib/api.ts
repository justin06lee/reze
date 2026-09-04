import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type {
  Attachment,
  Grabbed,
  GrabTools,
  Imported,
  Library,
  LoginItemState,
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
export const quit = () => invoke<void>("quit");
export const resizePalette = (height: number) => invoke<void>("resize_palette", { height });
export const setHotkeys = (palette: string, expand: string) =>
  invoke<void>("set_hotkeys", { palette, expand });
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
