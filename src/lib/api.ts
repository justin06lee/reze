import { invoke } from "@tauri-apps/api/core";
import type { Library, LoginItemState } from "./types";

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

/** Hide the palette and put `text` into the app the user was actually in. */
export const deliver = (text: string, copyOnly: boolean) =>
  invoke<void>("deliver", { text, copyOnly });

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
