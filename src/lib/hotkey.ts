/**
 * Translate a browser KeyboardEvent into the accelerator string Tauri's
 * global-shortcut plugin parses, and back into something readable.
 */

const MODIFIER_CODES = new Set([
  "MetaLeft",
  "MetaRight",
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
]);

export const isModifierOnly = (e: KeyboardEvent) => MODIFIER_CODES.has(e.code);

/** Returns null while only modifiers are held — the chord is not finished yet. */
export function accelerator(e: KeyboardEvent): string | null {
  if (isModifierOnly(e)) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Control");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Super");
  if (parts.length === 0) return null; // bare keys make terrible global hotkeys
  parts.push(e.code);
  return parts.join("+");
}

import { isMac } from "./platform";

const GLYPHS: Record<string, string> = {
  Control: "⌃",
  Ctrl: "⌃",
  Alt: "⌥",
  Option: "⌥",
  Shift: "⇧",
  Super: "⌘",
  Meta: "⌘",
  Command: "⌘",
  Cmd: "⌘",
  CmdOrCtrl: "⌘",
  CommandOrControl: "⌘",
};

/**
 * The same keys by name, as Linux and Windows write them. The glyphs are a
 * Mac convention, and ⌘ in particular means nothing on a keyboard without it.
 */
const NAMES: Record<string, string> = {
  Control: "Ctrl",
  Ctrl: "Ctrl",
  Alt: "Alt",
  Option: "Alt",
  Shift: "Shift",
  Super: "Super",
  Meta: "Super",
  Command: "Super",
  Cmd: "Super",
  CmdOrCtrl: "Ctrl",
  CommandOrControl: "Ctrl",
};

export function prettyHotkey(accel: string): string {
  if (!isMac) {
    return accel
      .split("+")
      .map((part) => NAMES[part] ?? keyName(part))
      .join("+");
  }
  return accel
    .split("+")
    .map((part) => {
      if (GLYPHS[part]) return GLYPHS[part];
      if (part.startsWith("Key")) return part.slice(3);
      if (part.startsWith("Digit")) return part.slice(5);
      if (part === "Space") return "Space";
      return part;
    })
    .join("");
}

function keyName(part: string): string {
  if (part.startsWith("Key")) return part.slice(3);
  if (part.startsWith("Digit")) return part.slice(5);
  return part;
}
