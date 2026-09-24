# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-24: Apple Pages, the flagship. The package reader, the lossless `pages-json` form, the document reader into the document model (`src/document`), the Word writer, and the Markdown event projection are in: `pages -> docx` matches Apple's own export paragraph for paragraph on 22 of 27 fixtures, and Pages reaches Markdown, HTML, text, and Markdown JSON through the model. Next: benchmarks note, then the 0.6.0 release. Map in `DOCS/formats/pages.md`, fixtures in `tests/fixtures/pages/`.
- 2026-09-23: Format roadmap in `DOCS/ROADMAP.md`: every tentative format by category with a status and a priority tier (S to D, mixing value, difficulty, and novelty), plus the keystones (inflate, XML, ZIP, PNG, protobuf) that unlock whole categories.

## Next

- 2026-09-24: Benchmarks note for `pages -> docx` and `pages -> markdown`; 0.6.0. Then Word input (the XML reader is in), which gives `docx -> markdown`, `html`, `text`, and `pages` later.
- 2026-09-23: HTML input, deferred behind Pages. Choose between HTML input (an HTML parser, unlocking HTML -> Markdown, text, and later DOCX and PDF) and Apple Pages (the flagship). See `ROADMAP.md`.

## Future

- 2026-09-22: External tier plumbing and ffmpeg wrapping for media.
- 2026-09-22: Type inference option for CSV -> JSON (opt-in, declared lossy).
- 2026-09-22: Pretty JSON output option.
- 2026-09-22: Delimiter options for CSV (TSV, semicolon).

## On Hold

- (none)

## Blockers

- (none)

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
  - No benchmark reference for `pages-json`: there is no Rust reader of the modern format to compare against, and the fixtures are small. Record throughput once a large real document is in hand.
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

- 2026-09-23: Smaller Markdown arenas (`u32` offsets in `Line`, boxed fence data in `Kind`) to cut first-touch page faults, which are now the largest single cost in the block parser on large inputs.
- 2026-09-23: Word-at-a-time scanning for the inline parser's special characters; the byte loop with a lookup table is its largest remaining cost.
- 2026-09-23: Unicode general-category tables for exact delimiter-run classification (see `DOCS/formats/markdown.md`, known deviations).
- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning with `std::arch`. The word-at-a-time scanner in `io::scan` closed the CSV -> JSON gap without intrinsics; the CSV reader alone is still slower than the `csv` crate reader (see benchmarks/csv-json.md), so this remains the stretch target.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Done

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
