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
  /** Name inside `~/.reze/assets`. Empty when `path` is set. */
  file: string;
  /** Absolute path to a file in a watched folder, referenced where it lies. */
  path?: string;
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
  /**
   * The watched folder this macro came from, if any.
   *
   * Derived macros are rebuilt from the folder on every scan, so editing one
   * would be editing something about to be overwritten — the editor shows them
   * read-only instead.
   */
  folder?: string;
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
  /** Folders whose files become macros. Downloads land in the first. */
  folders: string[];
  /** Watch typing so expand-in-place works in terminals and TUIs. */
  trackTyping: boolean;
  pasteMode: PasteMode;
  restoreClipboard: boolean;
  /** Opens the clipboard history. */
  clipboardHotkey: string;
  /** Whether what you copy is remembered at all. */
  clipboardHistory: boolean;
  /** How many copies to keep. */
  clipboardLimit: number;
};

export type Library = {
  version: number;
  settings: Settings;
  macros: Macro[];
  /** Use counts for folder macros, which cannot carry their own. */
  folderUsage?: Record<string, number>;
};

/** Which download tools are installed, and where. */
export type GrabTools = {
  ytDlp: string | null;
  galleryDl: string | null;
  ffmpeg: string | null;
};

/** A finished download, waiting to be named. */
export type Grabbed = {
  /** Where it landed, so the rename knows what to move. */
  path: string;
  /** The filename without its extension, to prefill the naming field. */
  suggestedName: string;
  attachment: Attachment;
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

/** The hotkey slots Settings can rebind. */
export type HotkeySlot = "hotkey" | "expandHotkey" | "clipboardHotkey";

/** Which preview a clipboard entry gets, and which of its fields carries it. */
export type ClipKind = "text" | "image" | "file";

/**
 * The app something was copied from.
 *
 * An inference — the pasteboard records no owner, so this is whoever was
 * frontmost when it changed — but it is the thing that makes a list of
 * near-identical snippets readable at a glance.
 */
export type ClipSource = {
  bundleId: string;
  name: string;
  /** Bare name inside `~/.reze/clipboard/icons`, written once per app. */
  icon?: string;
};

/**
 * One remembered copy.
 *
 * `preview` is the head of the text, not the text: the backend keeps the whole
 * thing and only resolves it when something is actually pasted, so listing two
 * hundred entries does not mean carrying two hundred documents into the window.
 */
export type ClipEntry = {
  id: string;
  kind: ClipKind;
  preview: string;
  /** Length of the whole text, so the preview can say what it left out. */
  chars: number;
  /** Bare name inside `~/.reze/clipboard`, for an image. */
  file?: string;
  /** Absolute paths, for files copied in Finder. */
  paths?: string[];
  mime: string;
  bytes: number;
  /** Unix milliseconds. */
  copiedAt: number;
  source?: ClipSource;
};

/** What kind of thing this was, for the line under its title. */
export function clipKindLabel(entry: ClipEntry): string {
  if (entry.kind === "image") return entry.mime === "image/tiff" ? "Image" : "PNG image";
  if (entry.kind === "file") {
    const n = entry.paths?.length ?? 0;
    return n > 1 ? `${n} files` : "File";
  }
  return "Text";
}

/**
 * When it was copied, the way a person would say it: the time if it was today,
 * the date if it was not.
 */
export function clipWhen(at: number): string {
  const then = new Date(at);
  const time = then.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
  const now = new Date();
  const sameDay =
    then.getFullYear() === now.getFullYear() &&
    then.getMonth() === now.getMonth() &&
    then.getDate() === now.getDate();
  if (sameDay) return time;
  const yesterday = new Date(now);
  yesterday.setDate(now.getDate() - 1);
  const wasYesterday =
    then.getFullYear() === yesterday.getFullYear() &&
    then.getMonth() === yesterday.getMonth() &&
    then.getDate() === yesterday.getDate();
  if (wasYesterday) return `Yesterday ${time}`;
  return `${then.toLocaleDateString([], { month: "short", day: "numeric" })} ${time}`;
}

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
