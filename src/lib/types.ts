export type Macro = {
  id: string;
  /** What you type in the palette, e.g. "full analysis". */
  name: string;
  description: string;
  tags: string[];
  /** The expansion. May contain {{vars}} and {{> includes}}. */
  body: string;
  usageCount: number;
};

export type PasteMode = "paste" | "copy";

export type Settings = {
  /** Opens the search palette. */
  hotkey: string;
  /** Expands the trigger already typed at the caret, with no window. */
  expandHotkey: string;
  /** Watch typing so expand-in-place works in terminals and TUIs. */
  trackTyping: boolean;
  pasteMode: PasteMode;
  restoreClipboard: boolean;
};

export type Library = {
  version: number;
  settings: Settings;
  macros: Macro[];
};

export const emptyMacro = (): Macro => ({
  id: crypto.randomUUID(),
  name: "",
  description: "",
  tags: [],
  body: "",
  usageCount: 0,
});
