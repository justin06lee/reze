// Writes what the desktop's own template language and fuzzy match make of a
// set of inputs, for the Swift port to be held to (`Parity.swift`).
//
//   bun ios/RezeKit/Tests/fixtures.ts        (or: make ios-fixtures)
//
// Run it again whenever `src/lib/template.ts` or `src/lib/fuzzy.ts` changes;
// if the Swift tests then fail, the port has fallen behind.

import { fuzzyMatch } from "../../../src/lib/fuzzy";
import { builtinsUsed, collectVariables, expandIncludes, render } from "../../../src/lib/template";
import type { Macro } from "../../../src/lib/types";

const macro = (name: string, body: string): Macro => ({
  id: name,
  name,
  description: "",
  tags: [],
  body,
  attachments: [],
  usageCount: 0,
});

const macros = [
  macro("greeting", "Hello {{name|there}}, it is {{date}}."),
  macro("Sign Off", "— {{author|me}}"),
  macro("letter", "{{> greeting}}\n\n{{body}}\n\n{{>sign off}}"),
  macro("loop a", "a then {{> loop b}}"),
  macro("loop b", "b then {{> loop a}}"),
  macro("self", "again: {{> self}}"),
  macro("lost", "see {{> nowhere }} and {{>  Sign Off  }}"),
];

const bodies = [
  "plain text, nothing to do",
  "{{name}} and {{ name | spaced default }} and {{name|second default}}",
  "{{topic|a|b}} keeps everything after the first pipe",
  "{{clipboard}} {{date}} {{time}} {{datetime}} {{ clipboard }}",
  "{{}} {{ }} {{! a note }} {{> greeting}}",
  "{{{triple}}} and {{un closed and }} stray braces {",
  "{{multi\nline|with a\nbreak}}",
  "{{|only a default}} {{name|}}",
  "émoji 🎉 {{who|wörld}} 日本語 {{誰}}",
  "{{> letter}}",
  "{{> loop a}}",
  "{{> self}}",
  "{{> lost}}",
  "{{>greeting}}{{>GREETING}}",
];

const values: Record<string, string>[] = [
  {},
  { name: "Justin", body: "The middle.", author: "" },
  { name: "", topic: "x", who: "you", 誰: "私", triple: "3", clipboard: "copied", date: "D", time: "T", datetime: "DT" },
];

const template = bodies.map((body) => {
  const expanded = expandIncludes(body, macros);
  return {
    body,
    expanded,
    variables: collectVariables(expanded.text),
    builtins: builtinsUsed(expanded.text),
    rendered: values.map((v) => render(expanded.text, v)),
  };
});

const targets = [
  "full analysis",
  "refactor plan",
  "commit message",
  "fa",
  "Full-Analysis_v2",
  "a/b.c:d",
  "",
  "rigor",
  "FULL ANALYSIS",
];
const queries = ["", "fa", "full", "fu an", "full analysis", "FA", "plan", "xyz", "a", "msg", "f  a", "r", "abcd", "-"];

const fuzzy = queries.flatMap((query) =>
  targets.map((target) => ({ query, target, match: fuzzyMatch(query, target) })),
);

const out = new URL("./RezeKitTests/Fixtures/parity.json", import.meta.url).pathname;
await Bun.write(out, JSON.stringify({ macros, values, template, fuzzy }, null, 2) + "\n");
console.log(`wrote ${template.length} template cases and ${fuzzy.length} fuzzy cases to ${out}`);
