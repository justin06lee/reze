import type { Macro } from "./types";

/**
 * The macro template language, such as it is:
 *
 *   {{name}}              a variable you get prompted for
 *   {{name|a default}}    same, pre-filled
 *   {{> other macro}}     inline another macro by name
 *   {{clipboard}}         built-ins, filled automatically, never prompted
 *
 * Deliberately tiny. Anything more expressive belongs in the prompt itself.
 */

const TOKEN = /\{\{\s*([^{}]*?)\s*\}\}/g;
const MAX_INCLUDE_DEPTH = 10;

export const BUILTINS = ["clipboard", "date", "time", "datetime"] as const;
export type Builtin = (typeof BUILTINS)[number];

const isBuiltin = (name: string): name is Builtin =>
  (BUILTINS as readonly string[]).includes(name);

export type Variable = {
  name: string;
  default: string;
};

export type ExpandResult = {
  text: string;
  /** Include targets that do not resolve to a macro — surfaced, not silently dropped. */
  missing: string[];
  /** Include names that form a cycle. */
  cyclic: string[];
};

const normalize = (s: string) => s.trim().toLowerCase();

/**
 * Splice `{{> name}}` includes into the body, depth-first.
 *
 * `trail` carries the include chain so a macro that (transitively) includes
 * itself is reported rather than recursing until the stack gives out.
 */
export function expandIncludes(
  body: string,
  macros: Macro[],
  trail: string[] = [],
  depth = 0,
): ExpandResult {
  const missing: string[] = [];
  const cyclic: string[] = [];

  const text = body.replace(TOKEN, (whole, inner: string) => {
    if (!inner.startsWith(">")) return whole;
    const target = inner.slice(1).trim();
    const key = normalize(target);

    if (trail.includes(key)) {
      cyclic.push(target);
      return `{{! cycle: ${target} }}`;
    }
    if (depth >= MAX_INCLUDE_DEPTH) {
      cyclic.push(target);
      return `{{! too deep: ${target} }}`;
    }

    const found = macros.find((m) => normalize(m.name) === key);
    if (!found) {
      missing.push(target);
      return `{{! missing: ${target} }}`;
    }

    const nested = expandIncludes(found.body, macros, [...trail, key], depth + 1);
    missing.push(...nested.missing);
    cyclic.push(...nested.cyclic);
    return nested.text;
  });

  return { text, missing, cyclic };
}

/** Variables the user still has to fill in — built-ins and includes excluded. */
export function collectVariables(text: string): Variable[] {
  const found = new Map<string, Variable>();
  for (const [, inner] of text.matchAll(TOKEN)) {
    const raw = inner.trim();
    if (!raw || raw.startsWith(">") || raw.startsWith("!")) continue;
    const pipe = raw.indexOf("|");
    const name = (pipe === -1 ? raw : raw.slice(0, pipe)).trim();
    const fallback = pipe === -1 ? "" : raw.slice(pipe + 1).trim();
    if (!name || isBuiltin(name)) continue;
    // First occurrence wins, so the default nearest the top of the macro is
    // the one the user sees.
    if (!found.has(name)) found.set(name, { name, default: fallback });
  }
  return [...found.values()];
}

export function builtinsUsed(text: string): Builtin[] {
  const used = new Set<Builtin>();
  for (const [, inner] of text.matchAll(TOKEN)) {
    const name = inner.trim();
    if (isBuiltin(name)) used.add(name);
  }
  return [...used];
}

/** Substitute values into a body that has already had its includes expanded. */
export function render(text: string, values: Record<string, string>): string {
  return text.replace(TOKEN, (whole, inner: string) => {
    const raw = inner.trim();
    if (!raw || raw.startsWith(">") || raw.startsWith("!")) return whole;
    const pipe = raw.indexOf("|");
    const name = (pipe === -1 ? raw : raw.slice(0, pipe)).trim();
    const fallback = pipe === -1 ? "" : raw.slice(pipe + 1).trim();
    const supplied = values[name];
    if (supplied !== undefined && supplied !== "") return supplied;
    return fallback;
  });
}

export function builtinValues(names: Builtin[], clipboard: string): Record<string, string> {
  const now = new Date();
  const out: Record<string, string> = {};
  for (const name of names) {
    switch (name) {
      case "clipboard":
        out.clipboard = clipboard;
        break;
      case "date":
        out.date = now.toISOString().slice(0, 10);
        break;
      case "time":
        out.time = now.toTimeString().slice(0, 5);
        break;
      case "datetime":
        out.datetime = now.toISOString();
        break;
    }
  }
  return out;
}
