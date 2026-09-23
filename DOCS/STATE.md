# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-23: Format roadmap in `DOCS/ROADMAP.md`: every tentative format by category with a status and a priority tier (S to D, mixing value, difficulty, and novelty), plus the keystones (inflate, XML, ZIP, PNG, protobuf) that unlock whole categories.

- 2026-09-23: Apple Pages -> DOCX native converter (flagship). Reverse-engineering notes will live in `DOCS/formats/pages.md`.

## Next

- 2026-09-23: Choose between HTML input (an HTML parser, unlocking HTML -> Markdown, text, and later DOCX and PDF) and Apple Pages (the flagship). See `ROADMAP.md`.

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

- 2026-09-23: The Markdown parser pushes events into a sink (`parse_into` and `EventSink`) as the production path; the iterator `Parser` stays for renderers that want to pull. Buffering events between parser and renderer cost more than rendering them.
- 2026-09-22: Writers own a 64 KiB buffer and hand sinks whole chunks. Per-field writes through `dyn Write` cost more than parsing did; buffering inside the writer was the single largest win in the first performance spike.
- 2026-09-22: Byte scanning is done eight bytes at a time with plain integer bit tricks before reaching for SIMD intrinsics. It is portable, dependency-free, and readable, and it was enough to beat the reference pipeline.
- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build. They run by hand on cause (a converter hot-path change, a release), never on a schedule, and every run is written up in `DOCS/benchmarks/<pair>.md` like a methods section.
