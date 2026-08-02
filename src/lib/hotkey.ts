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

export function prettyHotkey(accel: string): string {
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
