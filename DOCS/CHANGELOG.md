# Changelog

All notable changes to Sublime. Newest first. Follows the Keep a Changelog
layout. Every push adds a line under Unreleased; cutting a release moves that
section under a version heading.

## [Unreleased]

## [0.18.0] - 2026-09-26

Batch conversion: directories, globs, parallel workers, atomic writes,
and dry runs on the command line; the formats map as one graph per
category.

### Added

- Batch conversion: `sublime convert` takes several inputs, directories (`-r` to descend), and glob patterns (`*`, `?`, `**`, expanded by the tool where the shell does not), and writes each output named after its input with the target's extension, into `--out-dir` (or a trailing `dir/`) mirroring the input's structure, or beside the input. Files convert in parallel (`--jobs`, one per CPU by default); one file's failure does not stop the rest; files under a directory whose format is unknown are skipped and counted; `--dry-run` lists the plan without writing. Every output, batch or single, is written to a `.part` file and renamed into place, so a failed conversion leaves nothing behind and never disturbs an existing output. New events `file_started`, `file_failed`, and `batch_finished` in both log formats. Exit code is the worst outcome across the batch.
- `sublime convert a.csv b.csv --to tsv` is refused with a pointer to `--out-dir`: the old reading, "write a.csv's TSV over b.csv", would have destroyed an input.
- The binary size budget is raised 1.85 -> 1.9 MB for batch conversion (17 KB: threads, the directory walk, the glob matcher, and the part-file writer; a first draft on `std::sync::mpsc` cost 10 KB more and was replaced by a mutex and a condition variable).

### Changed

- The formats map is one Mermaid graph per category plus one for the crossings between categories, instead of a single graph of everything: a format with a converter into another category is a stadium in its own graph, and the other category is a hexagon in the crossing graph. The binary size budget is raised 1.8 -> 1.85 MB: the binary had 3 KB of headroom after 0.17.0 and this rendering takes 4 KB.

## [0.17.0] - 2026-09-25

The bridge between rows and documents: spreadsheets reach every
document format as tables, and document tables come out as rows; the
paths list doubles.

### Added

- The bridge between rows and documents (`src/io/csv/table.rs`, `src/converters/rows_document.rs`): `csv -> markdown` and `tsv -> markdown` turn rows into a Markdown table (first row the header, rows padded or cut to its width, line breaks in cells folded to spaces, everything GFM would misread escaped), so a spreadsheet reaches HTML, Word, text, Markdown JSON, and every document path through the planner; `markdown -> csv` and `markdown -> tsv` take a document's first table out as rows, so Word, Pages, and HTML tables reach every row and hub format. The paths list doubled (88 -> 184). Oracles in `tests/rows_document.rs`.
- Benchmark pair `csv-markdown` against Miller (`mlr --icsv --omd`, a real tool) with the csv crate and a bespoke table printer as context, and pulldown-cmark with the csv crate for the reverse, every line passing.

### Changed

- The formats map's Mermaid diagram draws an edge between categories to the other category's box (`csv -- conditional --> document`), since the hub behind the box carries it on to every format there; edges inside a category are unchanged.
- Markdown, for every path through it: the writer copies plain text in runs and scans for list-opening digits only where they can matter; the block parser keeps a table's cells in one vector with row ends instead of one vector per row (markdown -> csv on a million-row table 295 -> 215 MB); inline rendering hands plain text (no inline syntax, no autolink start) through as one event without the node machinery (markdown -> csv 108 -> 121 MB/s).

## [0.16.0] - 2026-09-25

Excel workbooks: a chosen sheet to CSV, TSV, JSON, and JSON Lines, and
CSV or TSV to a workbook, the first format built on the ZIP, XML, and
row work together.

### Added

- Excel workbooks: `src/io/xlsx/reader.rs` reads one sheet of a workbook into rows (the sheet list and relationships, shared strings with rich text, date detection from the cell styles, serial dates to ISO 8601, booleans, errors, formulas' cached values, gaps and the sheet dimension, the 1904 epoch) and `src/io/xlsx/writer.rs` writes rows into a one-sheet workbook streamed into the ZIP. Paths `xlsx -> csv`, `xlsx -> tsv`, `csv -> xlsx`, `tsv -> xlsx`, all conditional; JSON and JSON Lines reach through the row converters. `--sheet <name|number>` on `convert` picks the sheet read or names the sheet written. Oracles in `tests/xlsx_rows.rs` over hand-built fixtures in `tests/fixtures/xlsx`. Map in `DOCS/formats/xlsx.md`.
- Benchmark pair `xlsx-csv` against `calamine` with the `csv` crate (read) and the `csv` crate with `rust_xlsxwriter` (write), every line passing. Wasm size budget raised 850,000 -> 900,000 for the reader and writer.

## [0.15.0] - 2026-09-25

TSV and JSON Lines, and every row path among CSV, TSV, JSON, and JSON
Lines, all streamed in constant memory; the data category's planned set
is complete.

### Added

- TSV: the CSV reader and writer take a separator (`CsvReader::with_delimiter`, `CsvWriter::with_delimiter`, a tab scanner in `io::scan`), and the row converters are parametrized by it: `tsv -> json`, `json -> tsv`, `csv <-> tsv` (lossless, quoting for the target's separator), and TSV to and from JSON Lines. Extensions `.tsv` and `.tab`.
- JSON Lines: `jsonl -> json` (lossless) and `json -> jsonl` (an array becomes one line per element, any other root one line) through a token copier (`io::json::copy`) that never builds a tree; `csv -> jsonl`, `tsv -> jsonl`, `jsonl -> csv`, and `jsonl -> tsv` as the row converters' line mode. Extensions `.jsonl` and `.ndjson`. Errors carry the line.
- Oracles in `tests/rows.rs`; benchmark pairs `tsv-json` (against the `csv` crate with a tab delimiter) and `jsonl-json` (against `serde_json`'s stream deserializer), every line passing. Size budget raised 1.75 -> 1.8 MB for the two formats and twelve paths.

### Changed

- `CsvToJson` and `JsonToCsv` are values (`CSV_TO_JSON`, `TSV_TO_JSON`, `CSV_TO_JSONL`, `TSV_TO_JSONL`, and the reverse four) carrying their separator and row shape, not unit structs.

## [0.14.0] - 2026-09-25

Direct conversions between TOML, YAML, and XML, so the config trio and
XML reach each other in one step with nothing lost to a JSON hop.

### Added

- Direct pairs between the hub formats: `toml <-> yaml`, `toml <-> xml`, and `yaml <-> xml` (`src/converters/hub.rs`, one reader and one writer with no JSON in between). The planner now takes them instead of the two hops through JSON; the output is the same document, and infinities and NaN stay floats where JSON turned them into strings. Oracle in `tests/hub_pairs.rs`: every fixture through the direct pair and through JSON reads back to the same JSON. The pairs are the measured readers and writers back to back, so they carry no benchmark document of their own.

## [0.13.1] - 2026-09-25

The value hub as an arena tree: every TOML, YAML, and XML path uses a
third of the memory and reads up to twice as fast.

### Changed

- The value hub is an arena (`value::Tree`): 32-byte nodes chained by index, strings as spans into one text buffer, aliases copied by node with the text shared. TOML, YAML, and XML read into it and write from it, and JSON reads into it through the sink. Peak memory on the dense shapes fell 2.7 to 3.4 times (toml -> json 654 -> 237 MB, yaml -> json 615 -> 235 MB, xml -> json 570 -> 171 MB) and throughput rose 1.2 to 1.9 times (toml -> json 79 -> 148 MB/s, yaml -> json 82 -> 129 MB/s, xml -> json 104 -> 145 MB/s); every pair's rows are recorded in its benchmark document. `value::MemberIndex` now serves every table of a tree; the TOML and YAML readers use it instead of their own.

## [0.13.0] - 2026-09-24

XML both ways under the xmltodict mapping, with a strict streaming
reader; the data category now joins CSV, JSON, TOML, YAML, and XML, every
pair benchmarked ahead of its reference crate.

### Added

- XML as a data format: `src/io/xml/tree.rs` reads a document into the value hub under the mapping xmltodict and quick-xml share (elements as objects, attributes as `@name`, text as `#text`, repeated elements as arrays, every value a string), with a strict tokenizer that refuses what `xmllint` refuses and says where; `src/io/xml/writer.rs` writes the mapping back, pretty printed. Paths `xml -> json` and `json -> xml`, both conditional; TOML, YAML, and CSV reach XML through JSON. Oracles in `tests/xml_json.rs` over the fixtures in `tests/fixtures/xml`. Map in `DOCS/formats/xml.md`.
- Benchmark pair `xml-json` against `quick-xml` with `serde_json` doing the same mapping, dense and prose shapes. Size budget raised 1.7 -> 1.75 MB for the reader and writer.
- `value::MemberIndex`, the lazy per-table member index (a scan under sixteen members, a map above), shared by hub readers.

### Changed

- The XML reader pulls its input through a 256 KiB sliding window instead of holding the file: xml -> json peak memory on 194 MB of prose 505 -> 310 MB, and text runs, names, and whitespace are scanned a word at a time over the window (dense 90 -> 103 MB/s).

## [0.12.0] - 2026-09-24

YAML both ways: a YAML 1.2 core-schema reader and a block-style writer on
the value hub, so YAML, TOML, JSON, and CSV all reach each other;
benchmarked ahead of serde_yaml on every line, and both hub writers now
stream.

### Added

- YAML: `src/io/yaml/reader.rs` reads YAML 1.2 with the core schema (block and flow collections, the five scalar styles with folding and chomping, anchors and aliases, merge keys, tags, directives, multi-document streams, errors located by line and column) and `src/io/yaml/writer.rs` writes block-style YAML (strings plain when they read back unchanged, quoted otherwise, multi-line strings as literal blocks). Paths `yaml -> json` (conditional) and `json -> yaml` (lossless); TOML and CSV reach YAML through JSON. Oracles in `tests/yaml_json.rs` over the fixtures in `tests/fixtures/yaml`, plus every JSON fixture in the repository through YAML and back. Map in `DOCS/formats/yaml.md`.
- Benchmark pair `yaml-json` against `serde_yaml` with `serde_json`, dense and prose shapes. Size budgets raised for the reader and writer: binary 1.65 -> 1.7 MB, wasm 800,000 -> 850,000.

### Changed

- The tree-to-JSON walk with its loss reporting moved from the TOML converter into `io::json::from_value`, shared by every hub format; the TOML and YAML writers share one double-quoted string escaper in the hub.
- The TOML and YAML writers hand the sink 64 KiB chunks as they go (`value::ChunkedText`) instead of holding the whole document as one string: json -> yaml peak memory on 180 MB of prose 489 -> 299 MB.

## [0.11.0] - 2026-09-24

TOML, the first tree-shaped data format, both ways through a new value
hub that YAML and XML will share; benchmarked ahead of the `toml` crate
on every line.

### Added

- TOML: `src/io/toml/reader.rs` reads TOML 1.0 (every key, table, array-of-tables, inline table, string, number, and datetime form, with the definition rules enforced and errors located by line and column) and `src/io/toml/writer.rs` writes it (plain members before headers, inline arrays and tables, floats that read back as floats). Paths `toml -> json` and `json -> toml`, both conditional; CSV reaches TOML through JSON. Oracles in `tests/toml_json.rs` over the fixtures in `tests/fixtures/toml`. Map in `DOCS/formats/toml.md`.
- The value hub (`src/value`): the tree the tree-shaped data formats share, with a push-mode `ValueSink` for readers that can stream and a `TreeBuilder` for those that cannot. JSON reads into it (`io::json::parse_into`); the old `JsonValue` is gone.
- Benchmark pair `toml-json` against the `toml` crate with `serde_json`, dense and prose shapes, every line passing. Size budget raised 1.6 -> 1.65 MB for the reader, the writer, and the hub (27 KB).

### Changed

- TOML -> JSON reports a loss once per key path with array indices elided (`record[].created`), not once per row.
- `bench/run.sh` reads the peak-memory number from the last line GNU time writes, so a conversion that exits 2 (completed with loss) is measured instead of failing the row.

## [0.10.0] - 2026-09-24

HTML and plain text input, closing the document category's one-way
streets: every document format Sublime knows now reads and writes
through the hubs (Word, Markdown, HTML, text, Markdown JSON, with Pages
feeding them all).

### Added

- HTML input: `src/io/html/reader.rs` reads HTML into the Markdown event stream with a tokenizer and an element stack (no DOM): paragraphs, headings, quotes, code blocks with languages, lists (loose when the first item holds a paragraph), tables with alignment, links, images, emphasis, code spans, hard breaks, rules, cmark-gfm footnotes and task lists, full entity decoding, browser-style whitespace collapsing, and tag soup taken in stride (unclosed `p` and `li`, stray end tags, uppercase names, unquoted attributes); scripts, styles, and the head are skipped. Paths `html -> markdown`, `text`, `markdown-json`, `docx`. Oracles in `tests/html_document.rs`: the writer's HTML for 580 CommonMark and every GFM example reads back to itself, and a page of tag soup reads to the expected Markdown. Map in `DOCS/formats/html.md`.
- Plain text input: `src/io/text/reader.rs` reads paragraphs from runs of lines; `text -> markdown`, `html`, `docx`, `markdown-json`, declared conditional.
- Benchmark pairs `html-markdown` (against `htmd`), `html-text`, `html-docx`, and `text-markdown`. Size budgets raised for the two readers: binary 1.56 -> 1.6 MB, wasm 750,000 -> 800,000.

### Changed

- Word output from the events bridge streams: `DocxStream` writes the body one top-level block at a time as the builder closes it (`DocumentBuilder::streaming`), and the parts that depend on the whole (styles, numbering, footnotes, media) follow at the end. `markdown -> docx`, `html -> docx`, and `text -> docx` no longer hold the whole model; the Word writer's methods take the document as a parameter instead of holding it. Links are written as `HYPERLINK` fields, as Pages writes them, so a document of a hundred thousand links carries no relationship table (80 MB less on the dense benchmark shape); `numbering.xml` streams into the package in parts; and a ZIP entry over 1 MB is compressed in parts, since the compressor's chain table is four bytes per input byte. Dense `html -> docx` fell from 365 to 59 MB peak.

## [0.9.0] - 2026-09-24

Markdown into Word. The events bridge builds the document model from the
Markdown event stream, so Markdown and Markdown JSON reach Word with named
styles; 472 CommonMark and 21 GFM examples survive the bridge, and the
projection out of Word learned code, quotes, and rules by style name.

### Added

- Markdown into Word: `src/document/from_events.rs` builds the document model from the Markdown event stream (headings with outline levels, paragraphs, quotes by indent, code blocks as one paragraph of lines, code spans, lists with nesting and start numbers, tables with alignment, links, emphasis, footnotes, rules, images by data URI), with named styles Word users know; `markdown -> docx` and `markdown-json -> docx`, declared conditional (raw HTML dropped, other images become links, loose lists come out tight). Oracles in `tests/markdown_docx.rs`: 472 CommonMark and 21 GFM examples survive the bridge, and a document with every construct survives the trip through a Word file. Benchmark pair `markdown-docx` against pulldown-cmark feeding docx-rs. Size budget 1.53 -> 1.56 MB for the bridge, the converter, and the projection's formatting stack (binary 1,544,072 bytes).

### Changed

- The Markdown projection reads code blocks, quotes, and rules out of Word documents by their style names (`Source Code`, `Code`, `HTML Preformatted`, `Quote`, `Block Text`, `Horizontal Line`, and the character styles `Source Text`, `Code`, `HTML Code`, `Verbatim Char`); opens and closes inline formatting as a stack ordered by what the next run keeps, so `**foo *bar* baz**` no longer closes and reopens the bold around the italic; opens an empty list item as an item; and links bare email addresses by the GFM rules (`a@b-` is not one).

## [0.8.0] - 2026-09-24

Word input. A `.docx` reads into the document model, so Word reaches
Markdown, HTML, text, and Markdown JSON; proven against Apple's own Word
exports and benchmarked at 128 to 164 MB/s of uncompressed input on the
three pairs, every line passing.

### Added

- Word input: `src/io/docx/reader.rs` reads a `.docx` into the document model (styles with `basedOn` chains and document defaults, numbering with style links and start overrides, sections with headers and footers, footnotes and endnotes, tables with merged cells, inline and anchored pictures, text boxes, hyperlinks as elements and as fields, page fields, tracked changes, Office Math as text), giving `docx -> markdown`, `html`, `text`, and `markdown-json`. Proven against Apple's Word exports of the Pages fixtures (same text as the Pages documents on 22 of 28) and by reading our own Word output back to the same Markdown (`tests/docx_document.rs`). Map in `DOCS/formats/docx.md`. Size budget 1.43 -> 1.53 MB for the reader (binary 1,515,672 bytes); wasm budget 700,000 -> 750,000 (module 728,388).
- Benchmark pairs `docx-markdown`, `docx-html`, `docx-text` (the last against `docx-rs`), and the standard's rule for compressed inputs: throughput over the uncompressed bytes the reader parses.
- `DOCS/FORMATS.md` ends with a generated Mermaid map of the converters (`sublime paths --markdown`); the README gains an Install section and a binary-or-browser table.

### Changed

- The XML reader hands whitespace-only text nodes through (a Word run of one space is text); the MathML text helper skips them itself. The Markdown projection drops line breaks at the very end of a paragraph.

## [0.7.0] - 2026-09-24

Sublime in the browser: the converter ships as a WebAssembly module with every
release, built and smoke-tested from the same commit as the binaries.

### Added

- Sublime in the browser: `wasm/` builds the whole converter as a WebAssembly module (`sublime.wasm`, 681 KB, 282 KB gzipped) with a small ES module (`sublime.js`) that loads it and exposes `convert`, `formats`, `paths`, and `formatFor`, plus a working page (`index.html`). Every format and path the command line has runs on the visitor's machine. CI builds the module, runs its smoke test under node, and checks it against `wasm/size-budget`; every release publishes `sublime-<version>-wasm.zip` beside the binaries from the same commit, and the release workflow refuses a tag whose two crate versions differ.

### Changed

- The planner runs a multi-hop plan through in-memory buffers where there are no threads (`planner::execute_in_memory`, chosen on `wasm32`); the threaded chain moved to `planner::chain` and is compiled out there.

## [0.6.1] - 2026-09-24

A performance release for the Pages paths, measured pair by pair in
`DOCS/benchmarks/`: memory passes on every Pages row, Word passes every
goal under the compressed-output measure, a real resume converts to Word in
3 ms at 4.8 MB, and the text and JSON paths' throughput on the synthetic
dense shape stands at 30 to 40 MB/s against the 50 MB/s target, recorded as
a standing target with the structural levers named.

### Changed

- Table spans in the Pages reader are 16 bytes; the Snappy decoder copies overlapping runs in doubling chunks instead of a byte at a time.
- Benchmark standard: a path whose output is a compressed package (Word) measures throughput over the bytes it handles, input plus uncompressed output, since the compressor's work is proportional to what it must compress; the Word pair passes every goal under it (150 MB/s), and its rate per input byte stays as an extra row.
- The Pages reader reads its attribute tables with forward cursors instead of a binary search per run, copies a storage's text into the arena once, and splits ASCII paragraphs by byte scan; the Markdown projection caches resolved style chains. Dense-shape text paths 32 -> 40 MB/s. Size budget 1.42 -> 1.43 MB.
- On the document paths the attribute tables of a text storage stay encoded in the tree (`Tree::deferred`, `Node::Deferred`) and the reader parses them straight into vectors; dense-shape peak memory fell from 70 to 48 MB and every Pages document row now passes the memory goal. The Word writer interns repeated run formatting into character styles (`w:rStyle`), keeping the toggle properties inline; the dense-shape XML shrank from 19.3 to 15.6 MB. Size budget raised from 1.41 MB to 1.42 MB for the typed decode (binary 1,414,792 bytes).
- The document paths decode only the objects a document reaches (`Package::read_scope` with `Scope::Document`): references are followed from the document root, the package metadata, and the calculation engine, never through the stylesheet, theme, or view-state hubs. A real resume converts to Word in 3 ms at 4.8 MB peak (from 5 ms and 7 MB). The fast deflate level walks one candidate and skips indexing inside long matches; the Word XML no longer repeats sizes as `w:szCs` or marks every text node `xml:space`.
- The Word writer streams its body into the package in 256 KiB parts: `ZipWriter::begin_deflated`, `write_part`, and `end_deflated` write a deflated entry in sync-flushed parts with a data descriptor, and `deflate_part` with `Level::Fast` compresses each part. Word memory on the benchmark inputs fell from 175 to 75 MB (dense) and 94 to 45 MB (prose).
- The document model is a text arena with interned properties, links, revisions, images, and strings: a run is 64 bytes and owns no heap, `Document::paragraph_text` replaces `Paragraph::text`, and readers intern through the document. On the 55,000-paragraph benchmark input the text paths dropped from 142 to 75 MB peak and gained a third in throughput; Word from 232 to 175 MB. Size budget raised from 1.4 MB to 1.41 MB for it (binary 1,400,448 bytes after trimming).

### Added

- Every Pages pair is judged against the same two goals, 50 MB/s of input and 64 MB peak, defined in `pages-json.md`, since no peer reads the format; the two directions of a pair are never judged against each other. The Pages documents are rewritten to the template with those goals.
- Benchmark standard (`DOCS/benchmarks/README.md`): the same throughput and peak memory rows, units, and reference wording in every pair document, a Latest line at the top of each, and binary size and startup moved to `binary.md` per release.
- Benchmark pairs for Pages (`pages-json`, `pages-docx`, `pages-markdown`, `pages-html`, `pages-text`): a generator that scales a fixture into a large package through the lossless JSON form, a decompression floor for the package layer, one script and one document per pair, and a first results block each. The document paths fail their lines; the causes and the 0.6.1 plan are in `STATE.md`.

### Changed

- The Pages reader is no longer quadratic in the paragraph count: the attribute tables are binary-searched and UTF-16 offsets come from a cursor. A 55,000-paragraph document went from 16.9 s to 0.26 s. The Word writer no longer copies its body before compressing it.
- `bench/run.sh` compares the gnu binary against `size-budget` (what CI checks) and records the musl release asset's size beside it.

- Fonts a document asked for but the Mac lacked are written by their requested name (`compatibility_font_name`), as Pages exports them; PostScript font names are split into families generally (`ComicSansMS` -> `Comic Sans MS`, `AvenirNext-DemiBold` -> `Avenir Next`) instead of by a short list. The `fonts` fixture joins the Word comparison suite.

## [0.6.0] - 2026-09-24

### Added

- The document model (`src/document`): the hub between document formats, holding styles, sections with page setup, headers and footers, columns, paragraphs, runs, lists, tables, images, floating objects, footnotes, fields, tracked changes, and equations.
- Pages document reader: body text with paragraph and character styles resolved through the stylesheet, direct formatting from variation styles, lists, links, footnotes, page and section breaks, tables (cell storage, merged regions from the calculation engine, fills, header rows, column widths and row heights), inline and anchored images from the package's data files, floating text boxes and images (groups flattened), headers and footers for first, even, and odd pages, page setup, column layouts, tables of contents with page numbers, page-number and page-count fields, tracked insertions and deletions with their authors, and equations as MathML.
- Word writer and `pages -> docx` (and `pages-json -> docx`): styles by their Pages names, numbering, footnotes, headers and footers with their own parts, media parts with content types, tables with grid spans and vertical merges, inline and anchored pictures, text box shapes, page fields, section breaks (continuous for column changes), tracked changes as `w:ins` and `w:del`. Matches Apple's own export paragraph for paragraph on 23 of 28 fixtures (CI).
- XML reader (`io::xml`) for the tests and future HTML and Word input.
- Benchmark note for the Pages document paths (`DOCS/benchmarks/pages-docx.md`): every path a few milliseconds on a real resume, under 9 MB peak.
- The Markdown writer emits autolinks (`<https://…>`, `<user@host>`) for links whose text is their own address.
- The document model as a Markdown event stream (`document::markdown`), and with it `pages -> markdown`, `pages -> html`, `pages -> text`, and `pages -> markdown-json`: headings by outline level, bullet and numbered lists by level, bold, italic, and strikethrough, links, images by their package file name, tables as Markdown tables (merged cells as empty cells), footnotes as definitions, tracked changes accepted, equations as their text, text boxes after the body in page order. Headers, footers, and page layout are dropped, as Markdown has none.

### Changed

- Size budget raised from 1.2 MB to 1.4 MB for the document pipeline (release binary 1.36 MB).

## [0.5.1] - 2026-09-24

### Changed

- Deflate's Huffman builder sorts by key, which the current clippy requires; no behavior change.

## [0.5.0] - 2026-09-23

### Added

- Snappy compressor (greedy hash matcher). Rebuilt Pages packages are now Snappy-compressed in 64 KiB chunks and come out slightly smaller than Pages' own files (2,154,070 vs 2,164,388 bytes of streams across the fixtures; a real document rebuilt 702 bytes smaller than its original).
- Deflate compressor (32 KiB window, hash chains, dynamic Huffman blocks with a stored fallback) and `ZipWriter::add_deflated`. Ratio and speed are in zlib level 6's class.
- Type 10016 named `TP.UserDefinedGuideMapArchive`, settled from its payload shape against the Pages proto.

### Changed

- Base64 decodes through a lookup table and encodes four bytes at a time; schema field slots are found by a scan for small messages.

## [0.4.1] - 2026-09-23

### Changed

- Pages object trees are one arena per stream (sibling-linked 32-byte entries, strings and bytes as ranges into one buffer) instead of a heap node per field, and `pages-json` is read as a token stream straight into the arena. On the largest fixture, Pages -> JSON went from 16 ms to about 11 ms and JSON -> Pages from 41 ms to 18 ms, with allocation gone from the profile.

## [0.4.0] - 2026-09-23

### Added

- Apple Pages: `pages` <-> `pages-json` converters (native, lossless at the object level). The package is decoded into schema-named object trees that re-encode byte for byte; every fixture round-trips in CI. Formats `pages` (pages) and `pages-json` (no extension).
- Keystones, all zero-dependency: protobuf wire reader and schema-driven tree decoder and encoder, Snappy block decoder and literal encoder, inflate, CRC-32, ZIP reader and stored-entry writer, IWA stream reader, base64, a JSON value tree.
- Pages message schemas (658 messages) and type registry compiled in as packed tables, generated by `scripts/gen-pages-schema.py` from the community's reverse-engineered definitions.
- `sublime inspect`, in `--features dev-tools` builds only: dumps an iWork package's object graph with named fields.
- Pages fixture set: twenty-seven documents with Apple's Word, PDF, and text exports, generated on a Mac by `scripts/pages-fixtures/`.
- `DOCS/formats/pages.md`: the living map of the format.

### Changed

- Size budget raised to 1,200,000 bytes (release binary 1,130 KB: 184 KB of schema tables, about 90 KB of keystone and converter code).
- License: AGPL-3.0-or-later from this point on (releases 0.1.0 to 0.3.0 stay Apache-2.0). Contributions are accepted under Apache-2.0. See `LICENSING.md`.

## [0.3.0] - 2026-09-23

### Added

- Markdown writer (`io::markdown::writer`): events back to Markdown in one canonical form; every specification example round-trips to the same events in CI. Markdown is now a middle node for any format that reaches the event stream.
- Markdown -> plain text converter (native, lossy) and format `text` (txt): readable text with list markers, aligned tables, `text (url)` links, and numbered footnotes.
- Markdown <-> `markdown-json` converters (native, lossless): the event stream as JSON, one object per event, streamed both ways. The format has no extension; select it with `--to`/`--from`.
- Benchmark pairs `markdown-text` and `markdown-json` with their methods documents.
- `DOCS/ROADMAP.md`: tentative formats by category with status and priority tier, keystone building blocks, and a tier list.

### Changed

- JSON tokenizer copies runs of plain string bytes a word at a time (also speeds up JSON -> CSV).
- Converters share `converters::input::read_text_document` for whole-document UTF-8 input.
- Size budget raised to 900,000 bytes (release binary 842 KB after the three Markdown renderers).

## [0.2.0] - 2026-09-23

### Added

- Markdown parser (`io::markdown`): full CommonMark 0.31.2, GFM tables, strikethrough, task lists, autolink literals, tag filter, and footnotes, emitting an event stream. All specification examples run as CI tests.
- HTML writer (`io::html`) rendering Markdown events byte for byte like cmark, with a streaming mode.
- Markdown -> HTML converter (native, lossless). Formats `markdown` (md, markdown) and `html` (html, htm).
- Benchmark pair `markdown-html` against `pulldown-cmark`.
- `DOCS/formats/markdown.md`: parser design, corpus policy, known deviations.
- Formats carry a category (data, document, image, audio, video, archive) shown by `sublime formats` and used to group the formats reference. Code layout stays flat per format.
- Push-mode parsing: `markdown::parse_into` hands events to an `EventSink` as they are made; `HtmlWriter` is one, so the converter never buffers events. Text borrows the source wherever possible.

### Changed

- Size budget raised to 809,000 bytes for the Markdown subsystem (entity table, parser, writer).
- `Input` forwards `read_to_end` and `read_to_string`, so a file input is read into a buffer sized from its length.
- Word-at-a-time scanners (`io::scan`) share one loop; the tail of a slice is loaded without a stack round trip, which was stalling every short text run.

- Benchmarks: one document per pair in `DOCS/benchmarks/` with a methods-section template; harness split into `bench/src/pairs/` modules and `bench/pairs/` scripts, run as `bench/run.sh <pair>`.

## [0.1.0] - 2026-09-22

### Added

- Repository scaffold: crate, release profile, docs skeleton, size budget.
- Format declarations with detection by id, extension, and magic bytes.
- Converter trait, streaming input abstraction with rewind, fidelity and tier types, typed event system with sinks and conversion report.
- Streaming CSV reader with BOM, CRLF, quoting, and blank-line handling.
- CSV writer with minimal quoting.
- Streaming JSON tokenizer with full escape handling and verbatim numbers.
- Compact JSON writer.
- CSV -> JSON converter (native, lossless).
- JSON -> CSV converter (native, conditional): two-pass, constant memory from files.
- Converter registry with uniqueness tests.
- Planner: Dijkstra over the format graph with fidelity costs, --strict pruning, and --via waypoints.
- Multi-hop execution through in-memory pipes with per-hop timing events.
- Hand-written CLI argument parser with help text.
- Human and JSON-lines event renderers.
- CLI: convert, check, formats, paths, version; stable exit codes; JSON output mode.
- Fixture corpus and subprocess CLI tests covering every command and exit code.
- Generated formats reference, contributing guide with the drop-in recipe, docs drift check script.
- CI: fmt, clippy, doc, tests on Linux/macOS/Windows, formats doc drift, size budget, dependency ledger.
- Release workflow: tag verification, five static targets, checksums, changelog-based notes.
- Benchmark harness in `bench/` comparing against the `csv` and `serde_json` pipelines, with reader-only modes for both sides; results recorded in `DOCS/BENCHMARKS.md`.
- Word-at-a-time byte scanner (`io::scan`) used by the CSV reader and JSON writer; no SIMD intrinsics, no dependencies.

### Changed

- JSON writer buffers 64 KiB internally and hands the sink whole chunks; `into_inner` now flushes and returns `io::Result<W>`, and callers must `flush()` before reading the sink.
- CSV reader copies unquoted and quoted runs in bulk instead of one byte per call.
- CSV -> JSON pre-escapes header keys once per conversion (`JsonWriter::prepare_key`).
- Benchmark startup measurement uses the median of isolated runs and prints the process-spawn baseline.
