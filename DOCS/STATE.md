# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-26: The image category opens with an 8-bit pixel hub (`src/image`), PNG both ways (`src/io/png`: every depth, color type, palette, transparency, and interlace read in pieces as the file arrives; 8-bit written with adaptive filters), and BMP both ways (`src/io/bmp`). Maps in `DOCS/formats/png.md` and `bmp.md`, oracles in `tests/png_suite.rs` (PngSuite against Pillow) and `tests/png_bmp.rs`, pair `png-bmp` against the `png` and `image` crates. Underneath: a resumable inflater, a faster CRC-32, and a deflate matcher that spends its chain budget where matches pay.
- 2026-09-25: Batch conversion on the command line: many inputs, directories, globs, parallel workers, atomic `.part` writes, dry runs, per-file failure isolation.
- 2026-09-25: The rows-to-document bridge: CSV and TSV become a Markdown table and so reach every document format; a document's first table comes out as rows. Oracles in `tests/rows_document.rs`, pair `csv-markdown` against Miller.
- 2026-09-25: Excel workbooks, one sheet at a time: a reader that turns a chosen sheet into rows (shared strings, styles for dates, the 1904 epoch, gaps and dimension) and a writer that streams rows into a one-sheet workbook; `--sheet` on `convert`. Map in `DOCS/formats/xlsx.md`, oracles in `tests/xlsx_rows.rs`, pair `xlsx-csv` against `calamine` and `rust_xlsxwriter`.
- 2026-09-25: TSV and JSON Lines, and every row path among CSV, TSV, JSON, and JSON Lines, all streamed in constant memory. Oracles in `tests/rows.rs`, pairs `tsv-json` and `jsonl-json`.
- 2026-09-25: Direct pairs between TOML, YAML, and XML (`src/converters/hub.rs`), so the planner no longer routes them through JSON; infinities and NaN survive as floats. Oracle in `tests/hub_pairs.rs`.
- 2026-09-25: The value hub is an arena tree (`src/value/tree.rs`): the three hub formats cost about two to three bytes of memory per input byte where they cost ten, and read 1.2 to 1.9 times faster. Every pair document carries the new block.
- 2026-09-24: XML both ways through the value hub under the xmltodict mapping: a strict reader with located errors and a pretty-printing writer. Map in `DOCS/formats/xml.md`, oracles in `tests/xml_json.rs`, pair `xml-json` against `quick-xml`.
- 2026-09-24: YAML both ways through the value hub: a YAML 1.2 core-schema reader (block and flow, all scalar styles, anchors, merge keys, tags, multi-document) and a block-style writer. Map in `DOCS/formats/yaml.md`, oracles in `tests/yaml_json.rs`, pair `yaml-json` against `serde_yaml`.
- 2026-09-24: Data category, first tree format: TOML both ways through the new value hub (`src/value`: a `Value` tree, a push `ValueSink`, a `TreeBuilder`), with JSON reading into the hub. Map in `DOCS/formats/toml.md`, oracles in `tests/toml_json.rs`, pair `toml-json` passing every line against the `toml` crate.
- 2026-09-24: HTML and plain text input (phases 3 and 4): the HTML reader and the text reader emit the Markdown event stream, so a page or a text file reaches Markdown, text, Markdown JSON, and Word. Maps in `DOCS/formats/html.md`; oracles in `tests/html_document.rs`; pairs `html-markdown`, `html-text`, `html-docx`, `text-markdown`.
- 2026-09-24: Markdown into Word (phase 2): the events bridge builds the document model from the Markdown event stream, so every text input reaches Word; `markdown -> docx`, `markdown-json -> docx`. Oracles in `tests/markdown_docx.rs`, pair `markdown-docx`.
- 2026-09-24: Word input (phase 1 of the document-category plan under Next): the Word reader fills the document model and every projection out of it now takes `.docx`. Map in `DOCS/formats/docx.md`, oracles in `tests/docx_document.rs`, pairs `docx-markdown`, `docx-html`, `docx-text`.
- 2026-09-24: Sublime in the browser. `wasm/` is a second crate that exports the converter as a WebAssembly module with a small JavaScript loader (`wasm/README.md`); CI builds and smoke-tests it and every release publishes `sublime-<version>-wasm.zip`. The module is a first-class export: every library change ships in it, and its size is budgeted in `wasm/size-budget` the way the binary's is.
- 2026-09-24: Apple Pages, the flagship. The package reader, the lossless `pages-json` form, the document reader into the document model (`src/document`), the Word writer, and the Markdown event projection are in: `pages -> docx` matches Apple's own export paragraph for paragraph on 23 of 28 fixtures, and Pages reaches Markdown, HTML, text, and Markdown JSON through the model. Released as 0.6.0, with the performance work as 0.6.1 and the WebAssembly module as 0.7.0. Map in `DOCS/formats/pages.md`, fixtures in `tests/fixtures/pages/`.
- 2026-09-23: Format roadmap in `DOCS/ROADMAP.md`: every tentative format by category with a status and a priority tier (S to D, mixing value, difficulty, and novelty), plus the keystones (inflate, XML, ZIP, PNG, protobuf) that unlock whole categories.

## Next

The document category's one-way streets are closed (0.8.0 to 0.10.0) and the data category's planned set is in (0.11.0 to 0.16.0). What follows, in order:

- 2026-09-25, spreadsheets as a family (the XLSX reader and writer are the model; every entry below reads into rows and writes from them so it reaches CSV, TSV, JSON, JSON Lines, and the hub formats through the planner):
  - **Apple Numbers** (`.numbers`, Mac session): IWA package like Pages, so the Snappy and protobuf readers and the Pages fixtures pipeline apply; tables of the first sheet to rows first, then sheet and table selection through `--sheet`. Needs Numbers-made fixtures with Numbers' own CSV exports as references, which only the Mac can produce. Reader before writer; a writer needs the same object-graph work as the Pages writer and waits on that decision.
  - **OpenDocument spreadsheet** (`.ods`): the same keystones as XLSX (ZIP, XML) with `content.xml` rows and `table:table-cell` repeats; LibreOffice's own CSV exports as references. Reader and writer, a week.
  - **Google Sheets** is not a file format: it lives in Google's cloud and exports as XLSX, CSV, or ODS, which we read. Nothing to build; the docs should say so.
  - **Spreadsheet to document** (done 2026-09-25): rows become a Markdown table and reach every document format; a document's first table comes back as rows. Still open: a whole workbook as one JSON object of sheets (or one CSV per sheet) instead of one sheet per run, and a document's every table (not only the first) when the target can hold them.
  - **Excel-made fixtures** for the XLSX reader (Mac session, Excel or Numbers export), and number formats beyond dates.
- 2026-09-26, the image category after PNG and BMP: GIF read (LZW, the first frame) then JPEG baseline decode and encode with `--quality`, progressive decode after; GIF write (quantization) later. Every codec lands in the pixel hub and ships with a pair against a real crate.
- 2026-09-25, the cheap hub batch: INI, plist (XML and binary), MessagePack, CBOR, each a day on the value tree.
- Then the document category: ODT and EPUB, which reuse the Word and HTML work almost entirely; RTF; then PDF write as its own plan.
- The Pages writer decision stays with the Mac session.

## Future

- 2026-09-22: External tier plumbing and ffmpeg wrapping for media.
- 2026-09-22: Type inference option for CSV -> JSON (opt-in, declared lossy).
- 2026-09-22: Pretty JSON output option.
- 2026-09-22: Delimiter options for CSV on the command line (semicolon, pipe); TSV shipped as its own format on 2026-09-25.

## On Hold

- (none)

## Blockers

- 2026-09-26: `png -> bmp` on the photo shape misses the pass line against the `png` crate: 358 against 433 MB/s of decoded pixels in the recorded block, 2 to 17% behind across the session's runs (`DOCS/benchmarks/png-bmp.md`); the other seven lines pass, the flat decode and both encodes with margin. No tag until it passes or the pair is left out. In-process the decode is 95 ms against the crate's 78 on 61 MB of pixels: the inflater's literal path (69 ms, a table lookup per one to three literals with an 8-cycle dependent chain) against fdeflate, and the checksums and unfilter, which the crate does with SIMD intrinsics the main crate cannot use (`unsafe` is forbidden; only auto-vectorization is open, and the Adler-32 loop does not vectorize on the baseline x86-64 target, which has no 32-bit vector multiply). Levers left: a 13-bit table (measured 3%), Adler-32 as 16-bit lanes the baseline can multiply, and a `target-cpu` baseline decision (x86-64-v2 would open SSE4.1 to the compiler for every hot loop; it is a distribution decision, not a code change).
- 2026-09-24: The Pages document paths fail the shared Pages goals of 50 MB/s of input and 64 MB peak (`DOCS/benchmarks/pages-docx.md`, `pages-markdown.md`, `pages-html.md`, `pages-text.md`). After the typed decode of the attribute tables and the interned Word styles (on top of the slim model, the streamed Word body, and the reachable decode): memory passes on every document row (28 to 46 MB), Word passes throughput under the standard's compressed-output measure (150 MB/s of input plus uncompressed output), and the text paths fail throughput at 32 to 40 MB/s against 50. A real resume converts in 3 ms at 4.8 MB, inside both goals. What remains on the dense shape, in-process: package decode 10 ms (7.5 of it Snappy), document build 25 ms, the text writers 11 ms; wall clock adds about 10 ms of process start, file I/O, and first-touch page faults (13,000 pages). The text paths are 5 to 8 ms from the line; levers under Spikes. Causes measured: a 224-byte run struct with cloned font and language strings (48 MB of model for 170,000 runs), the Word body held whole before Deflate (30 MB), and the package decoding 570 objects to use 70. Fix for 0.6.1: intern strings in the style table and slim the run, stream the Word body through the compressor, decode objects on lookup.
- 2026-09-24: `pages -> pages-json` misses the 50 MB/s goal at 28 and 40 MB/s of package bytes (`DOCS/benchmarks/pages-json.md`); the reverse direction passes at 330 MB/s. Levers: the typed decode of the attribute tables written as JSON directly (the Spikes entry below), then `Tree::decode` itself and the JSON writer's per-field work.

## Tech Debt

- 2026-09-26: Image leftovers:
  - The hub is 8-bit: 16-bit PNG samples lose their low byte (reported as a loss). A 16-bit hub is the change if a lossless 16-bit path is ever wanted.
  - No metadata is carried (gamma, ICC, text, physical size); APNG frames are not read; BMP RLE is refused.
  - The PNG writer's filter heuristic (smallest sum of residual magnitudes, the standard) chooses Average on the synthetic photo where fixed Sub compressed 20% smaller under the first generator's mirrored noise; with independent noise both give the same size. An entropy estimate chose the same. A real-photo corpus would settle whether a bias toward Sub is worth it.
  - Deflate is greedy (no lazy matching): 14.2% against zlib's 13.2% on 8 MiB of words; the PNG encode is level-6 speed with a level-5 class ratio on compressible images.
  - The matcher's chain budget drops to a twelfth while recent matches average under five bytes; on the noisy photo that is 4% of output size (33.6 against 32.2 MB) for a three-times faster encode. A budget of a sixth gave the crate's size at 1.4 times its time.
- 2026-09-25: Excel leftovers:
  - The reader inflates the sheet part whole before parsing (415 MB peak on a 39 MB workbook whose sheet inflates to 326 MB); a windowed inflate feeding the XML reader would make it constant. The writer already streams.
  - One sheet per run; a whole-workbook form (a JSON object of sheets, or one CSV per sheet) is not offered.
  - Number formats other than dates are dropped; merged cells read as their top-left value.
  - No Excel-made fixtures yet; the set is hand-built from the specification.
- 2026-09-25: Value hub leftovers:
  - The TOML and YAML readers hold the input text whole beside the tree (60 to 190 MB of their peaks on the benchmark shapes); the XML reader's sliding window is the shape to port if a large config case ever matters.
  - `ValueSink` has one implementor (`TreeSink`) until a format streams.
  - TOML datetimes are validated for shape and range, not the calendar.
- 2026-09-24: WebAssembly leftovers:
  - Multi-hop paths hold each intermediate whole in memory (no threads in the browser); a single-hop path streams as on the command line. Fine for documents, a concern only for large data files through two hops.
  - The module has no size tooling beyond `opt-level = "z"`; `wasm-opt` would take 10 to 20% more off but is a toolchain dependency the build does not assume.
  - No progress or event reporting across the boundary; the host gets a status, the bytes, and one message.
- 2026-09-24: Pages document reader and Word writer leftovers:
  - Equations are written as MathML text; Office Math (OMML) from MathML is the proper output.
  - Floating objects anchor to the first paragraph on their page by counting explicit page breaks; pages that begin by overflow are not known without layout.
  - PDF and other non-picture media are left out of the Word file; converting PDF vector images to PNG needs a rasterizer.
  - Text box fills are solid colors only; gradients, image fills, strokes, and shape geometry other than rectangles are dropped. Lines and charts are dropped.
  - Comments (`table_highlight`) are not read.
  - Drop caps come out as an ordinary first letter (Apple splits them into a framed paragraph).
  - Header and footer areas (left, center, right) are joined as paragraphs; Word has no three-area header.
  - The release binary grew to 1.36 MB with the document pipeline (budget raised to 1.4 MB); std's backtrace symbolizer (gimli, addr2line, about 100 KB) is the largest non-feature cost and the lever if size matters.
- 2026-09-23: Pages leftovers from the package reader:
  - Deflate has no lazy matching; it is 7% larger than zlib level 9 on mixed data and level-6 class overall. Add lazy matching if a writer path needs the last percent.
  - 31 registry types without a schema; they decode raw. Resolve as the document reader needs them.
  - The schema comes from community protos of two vintages; fields Pages 12 added since decode raw. Coverage is measured by the round-trip test, not by name.
  - No peer reference for the Pages pairs: the harness scales fixtures into large documents and measures against the decompression floor and our own package round trip (`DOCS/benchmarks/pages-json.md`).
- 2026-09-23: Markdown leftovers, deliberately deferred in favor of the next flagship:
  - Text -> Markdown (paragraphs only, conditional). Do it when a second text-shaped input can share it.
  - Unicode general-category tables for exact delimiter-run classification; today an approximation that no corpus example reaches (`DOCS/formats/markdown.md`, known deviations). Costs binary size; do it when a real document hits it.
  - Label case folding covers a subset of Unicode; same trigger.
  - Vendor extensions (math, wiki links, description lists, front matter, heading attributes, superscript, emoji): each needs its own corpus; add one when a path needs it.
  - Dense-input parse speed is level with `pulldown-cmark`, not ahead; the arena-shrinking spike under Spikes is the known lever.
- 2026-09-22: Review minors deferred from the foundation build, none blocking:
  - CLI: (done 2026-09-25) every output is written to a `.part` file and renamed into place. Argument-parse errors are always human-formatted even with `--log-format json`. A stdout flush error is reported ahead of the command's own error. Markdown tables from `paths --markdown` do not escape `|`.
  - Planner: strict mode runs both Dijkstra passes even when the lossless one succeeds; a `--via` plus `--strict` failure names the waypoint as the destination in the error.
  - Execution: a converter thread panic is reported as `Unsupported`; `PipeReader::read` blocks on a zero-length buffer; partial output can reach stdout before a mid-chain error is reported (inherent to streaming, needs a doc note on `execute`).
  - JSON tokenizer: `skip_value` does not check bracket type agreement (`[1}` skips); a high surrogate followed by a non-escape consumes one extra byte before erroring.
  - JSON -> CSV: truncated input is reported as `Unsupported` rather than `Malformed`; duplicate keys within one object can mask differing-key detection.
  - CSV reader: bare CR accepted as a line ending but undocumented; error paths drop the record buffer's capacity.
  - Tests: no coverage for `Progress` events, `\b`/`\f` JSON escapes, `describe_path` with mixed multi-hop fidelity, or `bytes_consumed` with a BOM present.
- 2026-09-22: JSON -> CSV from stdin buffers the whole input to memory (two passes need a rewind). Files stream in constant memory. See Spikes.
- 2026-09-22: Multi-hop chains replay events after completion instead of streaming them live.

## Spikes

- 2026-09-24: Pages performance levers past the three phases, in order of expected return (numbers are the dense benchmark shape, in-process, from `DOCS/benchmarks/pages-docx.md`):
  - Done 2026-09-24: **typed decode of the attribute tables** (`Tree::deferred`, parsed by the reader): dense-shape memory 70 -> 48 MB (passes), text paths 27 -> 32 MB/s. **Run formatting interned into Word character styles**: XML 19.3 -> 15.6 MB, Word 17 -> 20 MB/s; the gain was smaller than projected because most runs carry only a named style or a toggle, which cannot move into a style.
  - Tried 2026-09-24 and kept for correctness, without measurable gain: a one-entry cache in front of the style hash lookups (removed), 16-byte table spans, bulk overlapping copies in the Snappy decoder. The text paths sit at 58 ms wall on the dense shape against 50: package decode 10 ms (7.5 Snappy), document build 25 ms, writer 11 ms, process and page faults about 10 ms. What remains is structural: a Snappy decoder inner loop at 1.5 GB/s (about 3 ms), a document without per-paragraph vectors (one run vector per document; fewer pages touched, cheaper drop), and a model-free streaming text path (rejected so far, see below).
  - **Merging adjacent runs with equal effective formatting.** Pages splits runs at style-object boundaries that often resolve to the same look; fewer runs means less of everything downstream.
  - **Dropping the object trees once the model is built**, for the document paths only; the media bytes are already copied out.
  - **Rendering Word straight from the storage tables without a model** was considered and set aside: it saves the model build (30 ms) at the cost of a second reader inside the writer and the loss of the hub architecture every other output depends on. Revisit only if the levers above leave the goal out of reach.
  - With the first two levers the text paths (`pages -> markdown`, `html`, `text`) project to about 45 ms on the dense shape (the 50 MB/s goal is 50 ms), Word to about 65 ms, and `pages -> pages-json` to about 40 MB/s dense and past the goal on prose; the goals stay as they are. Every Pages pair document's Conclusions names the rows it fails and the levers that apply to it.

- 2026-09-23: Smaller Markdown arenas (`u32` offsets in `Line`, boxed fence data in `Kind`) to cut first-touch page faults, which are now the largest single cost in the block parser on large inputs.
- 2026-09-23: Word-at-a-time scanning for the inline parser's special characters; the byte loop with a lookup table is its largest remaining cost.
- 2026-09-23: Unicode general-category tables for exact delimiter-run classification (see `DOCS/formats/markdown.md`, known deviations).
- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning with `std::arch`. The word-at-a-time scanner in `io::scan` closed the CSV -> JSON gap without intrinsics; the CSV reader alone is still slower than the `csv` crate reader (see benchmarks/csv-json.md), so this remains the stretch target.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Done

- 2026-09-26: Batch conversion released as 0.18.0.
- 2026-09-25: The rows-to-document bridge released as 0.17.0, with the Markdown pairs re-validated.
- 2026-09-25: Excel workbooks one sheet at a time released as 0.16.0.
- 2026-09-25: TSV and JSON Lines with every row path released as 0.15.0.
- 2026-09-25: Direct TOML, YAML, and XML pairs released as 0.14.0.
- 2026-09-25: Value hub as an arena tree released as 0.13.1: memory 2.7 to 3.4 times lower and throughput 1.2 to 1.9 times higher on every hub pair.
- 2026-09-24: XML both ways released as 0.13.0: `xml -> json`, `json -> xml`, the `xml-json` pair passing every line against `quick-xml`; the reader streams its input.
- 2026-09-24: YAML both ways released as 0.12.0: `yaml -> json`, `json -> yaml`, the `yaml-json` pair passing every line against `serde_yaml`; the TOML and YAML writers stream.
- 2026-09-24: TOML both ways and the value hub released as 0.11.0: `toml -> json`, `json -> toml`, the `toml-json` pair passing every line against the `toml` crate.
- 2026-09-24: HTML and plain text input released as 0.10.0 (document-category phases 3 and 4): the tag-soup HTML reader and the text reader into the event stream; `html -> markdown`, `text`, `markdown-json`, `docx` and `text -> markdown`, `html`, `docx`; four benchmark pairs.
- 2026-09-24: Markdown into Word released as 0.9.0 (document-category phase 2): the events bridge with named styles, `markdown -> docx` and `markdown-json -> docx`, corpus oracles, the `markdown-docx` pair.
- 2026-09-24: Word input released as 0.8.0 (document-category phase 1): the Word reader into the document model, `docx -> markdown`, `html`, `text`, `markdown-json`, oracles against Apple's exports and our own writer, three benchmark pairs passing every line.
- 2026-09-24: WebAssembly module released as 0.7.0: `wasm/` crate, `sublime.js` loader, demo page, node smoke test, CI job with a size budget, and `sublime-<version>-wasm.zip` in every release.
- 2026-09-24: Pages performance released as 0.6.1: slim document model, streamed Word body, reachable decode, typed attribute tables, interned Word styles, reader cursors; benchmark pairs and documents for every Pages path with the compressed-output standard.
- 2026-09-24: Pages document paths released as 0.6.0: `pages -> docx` matching Apple's export on 23 of 28 fixtures, and `pages -> markdown`, `html`, `text`, `markdown-json` through the document model.
- 2026-09-23: Markdown finished as 0.3.0: Markdown writer (round-trips every specification example), Markdown -> plain text, Markdown <-> events as JSON, all faster and leaner than the reference pipelines.
- 2026-09-23: Markdown -> HTML released as 0.2.0: full CommonMark plus GFM extensions and footnotes, all specification examples passing, faster and leaner than `pulldown-cmark` on the benchmark input.
- 2026-09-22: Foundation released as 0.1.0: framework, CSV <-> JSON, docs, CI, release pipeline, benchmark harness with all pass lines met.

## Decisions

- 2026-09-23: Licensed AGPL-3.0-or-later, contributions inbound under Apache-2.0, no CLA, no public commercial track. The goal is that nobody paywalls the work without sharing back; exceptions are handled privately on request (`LICENSING.md`). Releases 0.1.0 to 0.3.0 stay Apache-2.0.
- 2026-09-23: The Markdown parser pushes events into a sink (`parse_into` and `EventSink`) as the production path; the iterator `Parser` stays for renderers that want to pull. Buffering events between parser and renderer cost more than rendering them.
- 2026-09-23: Object trees are arenas: one entry vector and one byte buffer per stream, chains linked by index, schema fields resolved by index. A heap node per field cost a third of the conversion time; the arena is also the layout the document reader walks.
- 2026-09-23: Pages is read into the exact object graph first (`pages-json`, lossless, byte-for-byte re-encodable), and every document-level reader is a projection over that graph. The round trip is the correctness oracle for the package layer, so schema gaps can never lose data silently.
- 2026-09-23: Reverse-engineered schemas are compiled in as packed tables generated from the community's proto files, never as a runtime dependency; the generator records its sources.
- 2026-09-22: Writers own a 64 KiB buffer and hand sinks whole chunks. Per-field writes through `dyn Write` cost more than parsing did; buffering inside the writer was the single largest win in the first performance spike.
- 2026-09-22: Byte scanning is done eight bytes at a time with plain integer bit tricks before reaching for SIMD intrinsics. It is portable, dependency-free, and readable, and it was enough to beat the reference pipeline.
- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build. They run by hand on cause (a converter hot-path change, a release), never on a schedule, and every run is written up in `DOCS/benchmarks/<pair>.md` like a methods section.
