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

// Word input: Apple's own export of the lists fixture reads back as Markdown.
const word = new Uint8Array(readFileSync("tests/fixtures/pages/reference/lists.docx"));
const fromWord = sublime.convert(word, "docx", "markdown");
check(fromWord.status === "converted-with-loss", `docx -> markdown status ${fromWord.status}: ${fromWord.message}`);
check(new TextDecoder().decode(fromWord.bytes).startsWith("# Lists"), "docx -> markdown starts with the title");
check(new TextDecoder().decode(fromWord.bytes).includes("  - Nested bullet under the second\n"), "docx -> markdown keeps nested lists");

// HEIC: a tiled iPhone-style photo decodes without threads, to PNG and
// (as its own YCbCr) to JPEG.
const heic = new Uint8Array(readFileSync("tests/fixtures/heic/apple-photo.heic"));
const png = sublime.convert(heic, "heic", "png");
check(png.status.startsWith("converted") && png.bytes[1] === 0x50 && png.bytes.length > 100000, `heic -> png: ${png.status} ${png.message}`);
const jpg = sublime.convert(heic, "heic", "jpeg");
check(jpg.status.startsWith("converted") && jpg.bytes[0] === 0xff && jpg.bytes[1] === 0xd8, `heic -> jpeg: ${jpg.status} ${jpg.message}`);

const csv = new TextEncoder().encode("a,b\n1,2\n");
const json = sublime.convert(csv, "csv", "json");
check(new TextDecoder().decode(json.bytes).includes("\"a\""), "csv -> json");
check(sublime.convert(csv, "csv", "nope").status === "unknown-format", "unknown format id");
// Every format now reaches every other: rows reach Pages as a table.
const table = sublime.convert(csv, "csv", "pages");
check(table.status.startsWith("converted") && table.bytes[0] === 0x50 && table.bytes[1] === 0x4b, `csv -> pages: ${table.status} ${table.message}`);

// A document's tables into a one-table format come back as a ZIP of CSVs;
// into a workbook, as one workbook.
const tables = new TextEncoder().encode("# A\n\n| x |\n|---|\n| 1 |\n\n# B\n\n| y |\n|---|\n| 2 |\n");
const split = sublime.convert(tables, "markdown", "csv");
const zipped = new TextDecoder().decode(split.bytes);
check(split.parts === 2 && split.bytes[0] === 0x50 && split.bytes[1] === 0x4b && zipped.includes("A.csv") && zipped.includes("B.csv"), `markdown -> csv with two tables is a ZIP: ${split.status} ${split.message}`);
const sheets = sublime.convert(tables, "markdown", "json");
check(new TextDecoder().decode(sheets.bytes) === '{"A":[{"x":"1"}],"B":[{"y":"2"}]}', "markdown -> json keeps both tables");

console.log(`sublime.wasm: ${readFileSync(wasmPath).length} bytes; pages -> docx on text-styles.pages in ${elapsed.toFixed(1)} ms`);
if (failures.length) {
  console.error("smoke test failed:\n  " + failures.join("\n  "));
  process.exit(1);
}
console.log("smoke test passed");
