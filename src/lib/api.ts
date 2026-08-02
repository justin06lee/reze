import { invoke } from "@tauri-apps/api/core";
import type { Library } from "./types";

export const getLibrary = () => invoke<Library>("get_library");
export const saveLibrary = (library: Library) => invoke<void>("save_library", { library });
export const bumpUsage = (id: string) => invoke<void>("bump_usage", { id });

export const readClipboard = () => invoke<string>("read_clipboard");

export const accessibilityStatus = () => invoke<boolean>("accessibility_status");
export const requestAccessibility = () => invoke<boolean>("request_accessibility");
export const openAccessibilitySettings = () => invoke<void>("open_accessibility_settings");

/** Hide the palette and put `text` into the app the user was actually in. */
export const deliver = (text: string, copyOnly: boolean) =>
  invoke<void>("deliver", { text, copyOnly });

export const hidePalette = () => invoke<void>("hide_palette");
export const openEditor = () => invoke<void>("open_editor");
export const resizePalette = (height: number) => invoke<void>("resize_palette", { height });
export const setHotkey = (hotkey: string) => invoke<void>("set_hotkey", { hotkey });
export const libraryPath = () => invoke<string>("library_path");
export const revealLibrary = () => invoke<void>("reveal_library");
