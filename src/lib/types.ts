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
  hotkey: string;
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
