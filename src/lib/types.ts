/** Which preview to draw for an attachment. Derived from its MIME type. */
export type AttachmentKind = "image" | "video" | "audio" | "file";

/**
 * A file pasted along with a macro's body.
 *
 * `file` is a bare name inside `~/.reze/assets`, never a path — the backend
 * refuses anything else, so a hand-edited library cannot point at `~/.ssh`.
 */
export type Attachment = {
  id: string;
  file: string;
  /** What it was called when it came in, for display only. */
  name: string;
  kind: AttachmentKind;
  mime: string;
  bytes: number;
};

export type Macro = {
  id: string;
  /** What you type in the palette, e.g. "full analysis". */
  name: string;
  description: string;
  tags: string[];
  /** The expansion. May contain {{vars}} and {{> includes}}. */
  body: string;
  /** Files pasted along with the body — images, video, anything. */
  attachments: Attachment[];
  usageCount: number;
};

export type PasteMode = "paste" | "copy";

/**
 * Whether Reze starts itself at login, as the OS reports it.
 *
 * Deliberately absent from `Settings` — the system owns this, so it is read
 * back rather than remembered. `requiresApproval` means macOS has the
 * registration but the user switched it off in System Settings.
 */
export type LoginItemState = "enabled" | "disabled" | "requiresApproval" | "unsupported";

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
  attachments: [],
  usageCount: 0,
});

/** What an import produced, and why anything else was skipped. */
export type Imported = {
  attachments: Attachment[];
  errors: string[];
};

/** Human-sized file size. Decimal units, because that is what Finder shows. */
export function formatBytes(bytes: number): string {
  if (bytes < 1000) return `${bytes} B`;
  const units = ["kB", "MB", "GB", "TB"];
  let value = bytes / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  return `${value < 10 ? value.toFixed(1) : Math.round(value)} ${units[unit]}`;
}
