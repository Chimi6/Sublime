# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-24: Apple Pages, the flagship. The package reader, the lossless `pages-json` form, the document reader into the document model (`src/document`), the Word writer, and the Markdown event projection are in: `pages -> docx` matches Apple's own export paragraph for paragraph on 23 of 28 fixtures, and Pages reaches Markdown, HTML, text, and Markdown JSON through the model. Released as 0.6.0, with the performance work as 0.6.1. Map in `DOCS/formats/pages.md`, fixtures in `tests/fixtures/pages/`.
- 2026-09-23: Format roadmap in `DOCS/ROADMAP.md`: every tentative format by category with a status and a priority tier (S to D, mixing value, difficulty, and novelty), plus the keystones (inflate, XML, ZIP, PNG, protobuf) that unlock whole categories.

## Next

- 2026-09-24: Pages performance to the benchmark goals, in three phases, each its own PR with its own benchmark block: a slim document model (one text arena, interned properties, 32-byte runs), the Word body streamed through a faster deflate, and objects decoded on lookup. 0.6.1 when every Pages row passes. Then Word input (the XML reader is in), which gives `docx -> markdown`, `html`, `text`, and `pages` later.
- 2026-09-23: HTML input, deferred behind Pages. Choose between HTML input (an HTML parser, unlocking HTML -> Markdown, text, and later DOCX and PDF) and Apple Pages (the flagship). See `ROADMAP.md`.

## Future

- 2026-09-22: External tier plumbing and ffmpeg wrapping for media.
- 2026-09-22: Type inference option for CSV -> JSON (opt-in, declared lossy).
- 2026-09-22: Pretty JSON output option.
- 2026-09-22: Delimiter options for CSV (TSV, semicolon).

## On Hold

- (none)

## Blockers

- 2026-09-24: The Pages document paths fail the shared Pages goals of 50 MB/s of input and 64 MB peak (`DOCS/benchmarks/pages-docx.md`, `pages-markdown.md`, `pages-html.md`, `pages-text.md`). After the typed decode of the attribute tables and the interned Word styles (on top of the slim model, the streamed Word body, and the reachable decode): memory passes on every document row (28 to 46 MB), Word passes throughput under the standard's compressed-output measure (150 MB/s of input plus uncompressed output), and the text paths fail throughput at 32 to 40 MB/s against 50. A real resume converts in 3 ms at 4.8 MB, inside both goals. What remains on the dense shape, in-process: package decode 10 ms (7.5 of it Snappy), document build 25 ms, the text writers 11 ms; wall clock adds about 10 ms of process start, file I/O, and first-touch page faults (13,000 pages). The text paths are 5 to 8 ms from the line; levers under Spikes. Causes measured: a 224-byte run struct with cloned font and language strings (48 MB of model for 170,000 runs), the Word body held whole before Deflate (30 MB), and the package decoding 570 objects to use 70. Fix for 0.6.1: intern strings in the style table and slim the run, stream the Word body through the compressor, decode objects on lookup.
- 2026-09-24: `pages -> pages-json` misses the 50 MB/s goal at 28 and 40 MB/s of package bytes (`DOCS/benchmarks/pages-json.md`); the reverse direction passes at 330 MB/s. Levers: the typed decode of the attribute tables written as JSON directly (the Spikes entry below), then `Tree::decode` itself and the JSON writer's per-field work.

## Tech Debt

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
  - CLI: a failed `convert` removes a pre-existing output file (it was already truncated by create); write-to-temp-then-rename would be non-destructive. Argument-parse errors are always human-formatted even with `--log-format json`. A stdout flush error is reported ahead of the command's own error. Markdown tables from `paths --markdown` do not escape `|`.
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
