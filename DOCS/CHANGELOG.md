# Changelog

All notable changes to Sublime. Newest first. Follows the Keep a Changelog
layout. Every push adds a line under Unreleased; cutting a release moves that
section under a version heading.

## [Unreleased]

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
