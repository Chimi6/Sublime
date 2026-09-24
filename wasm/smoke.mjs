// Proves the module in the browser's execution model under node: loads it,
// lists formats and paths, converts fixtures, and checks the results.
// Run from the repository root after building the crate for wasm32.
import { readFileSync } from "node:fs";
import { Sublime } from "./sublime.js";

const wasmPath = new URL("./target/wasm32-unknown-unknown/release/sublime_wasm.wasm", import.meta.url);
const sublime = await Sublime.load(readFileSync(wasmPath));
const failures = [];
const check = (condition, what) => { if (!condition) failures.push(what); };

const formats = sublime.formats();
check(formats.some(f => f.id === "pages"), "formats lists pages");
check(sublime.paths().some(p => p.from === "pages" && p.to === "docx"), "paths lists pages -> docx");

const pages = new Uint8Array(readFileSync("tests/fixtures/pages/text-styles.pages"));
const started = performance.now();
const docx = sublime.convert(pages, "pages", "docx");
const elapsed = performance.now() - started;
check(docx.status === "converted-with-loss", `pages -> docx status ${docx.status}: ${docx.message}`);
check(docx.bytes[0] === 0x50 && docx.bytes[1] === 0x4b, "docx output is a ZIP");
check(docx.bytes.length > 2000, "docx output has content");

const markdown = sublime.convert(pages, "pages", "markdown");
const text = new TextDecoder().decode(markdown.bytes);
check(text.startsWith("# Text Styles"), "pages -> markdown starts with the title");

// A two-hop path runs sequentially in memory.
const events = sublime.convert(pages, "pages", "markdown-json");
check(events.status.startsWith("converted") && events.bytes.length > 100, `pages -> markdown-json: ${events.status} ${events.message}`);

const csv = new TextEncoder().encode("a,b\n1,2\n");
const json = sublime.convert(csv, "csv", "json");
check(new TextDecoder().decode(json.bytes).includes("\"a\""), "csv -> json");
check(sublime.convert(csv, "csv", "nope").status === "unknown-format", "unknown format id");
check(sublime.convert(csv, "csv", "pages").status === "no-path", "no path");

console.log(`sublime.wasm: ${readFileSync(wasmPath).length} bytes; pages -> docx on text-styles.pages in ${elapsed.toFixed(1)} ms`);
if (failures.length) {
  console.error("smoke test failed:\n  " + failures.join("\n  "));
  process.exit(1);
}
console.log("smoke test passed");
