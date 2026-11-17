// Pushes the hostile inputs of tests/fixtures/hostile (and deep nesting made
// here) through the module under node: each must end as the native binary
// does, a clean "failed" or a conversion, and never trap. Only here do the
// browser's smaller stack and 32-bit sizes get exercised. Run from the
// repository root after building the crate for wasm32.
import { readFileSync } from "node:fs";
import { Sublime } from "./sublime.js";

const wasmPath = new URL("./target/wasm32-unknown-unknown/release/sublime_wasm.wasm", import.meta.url);
const sublime = await Sublime.load(readFileSync(wasmPath));
const failures = [];

const run = (name, bytes, to, expect) => {
  const from = sublime.formatFor(name);
  const target = sublime.formatFor(`x.${to}`);
  const result = sublime.convert(bytes, from, target);
  const ended = result.status.startsWith("converted") ? "ok" : "error";
  if (result.crashed) {
    failures.push(`${name} -> ${to}: ${result.message}`);
  } else if (ended !== expect) {
    failures.push(`${name} -> ${to}: ${ended} (${result.message}), expected ${expect}`);
  }
};

const directory = "tests/fixtures/hostile";
for (const line of readFileSync(`${directory}/cases.txt`, "utf8").split("\n")) {
  if (!line.trim() || line.startsWith("#")) continue;
  const [name, to, expect] = line.trim().split(/\s+/);
  run(name, new Uint8Array(readFileSync(`${directory}/${name}`)), to, expect);
}

// Nesting past the traps measured in the audit (3,000 to 8,000 levels).
const DEEP = 20000;
const deep = (open, middle, close) => open.repeat(DEEP) + middle + close.repeat(DEEP);
const generated = [
  ["deep.json", deep("[", "", "]"), "yaml", "error"],
  ["deep-object.json", deep('{"a":', "1", "}"), "toml", "error"],
  ["deep-flow.yaml", deep("[", "", "]"), "json", "error"],
  ["deep-dash.yaml", "- ".repeat(DEEP) + "x\n", "json", "error"],
  ["deep-array.toml", "a = " + deep("[", "1", "]"), "json", "error"],
  ["deep-dotted.toml", "a.".repeat(DEEP) + "a = 1\n", "json", "error"],
  ["deep.xml", deep("<a>", "x", "</a>"), "json", "error"],
  ["deep-quote.md", ">".repeat(DEEP) + " x\n", "html", "ok"],
  ["deep-list.md", "- ".repeat(DEEP) + "x\n", "docx", "ok"],
];
for (const [name, text, to, expect] of generated) {
  run(name, new TextEncoder().encode(text), to, expect);
}

// After all of that the module still converts.
const small = new TextEncoder().encode("a,b\n1,2\n");
const csv = sublime.convert(small, "csv", "json");
if (csv.status !== "converted") failures.push(`csv -> json after the corpus: ${csv.status} ${csv.message}`);

// A trap (forced here: no input above traps) fails that call only; the
// instance is replaced and the next call converts.
sublime.exports = { ...sublime.exports, convert_with: () => { throw new WebAssembly.RuntimeError("unreachable"); } };
const trapped = sublime.convert(small, "csv", "json");
if (!trapped.crashed || trapped.status !== "failed") failures.push(`a trap was not reported: ${trapped.status}`);
await sublime.ready();
const after = sublime.convert(small, "csv", "json");
if (after.status !== "converted") failures.push(`csv -> json after a trap: ${after.status} ${after.message}`);

if (failures.length > 0) {
  console.error(`hostile: ${failures.length} failure(s)`);
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
console.log("hostile: every input ended cleanly");
