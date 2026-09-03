import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Attachment, Imported, Library, LoginItemState } from "./types";

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
 * actually in. Attachments are the bare file names stored on the macro.
 */
export const deliver = (text: string, copyOnly: boolean, attachments: string[]) =>
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
  return convertFileSrc(`${await assetsDir()}/${attachment.file}`);
}
