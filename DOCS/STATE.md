# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-22: Markdown -> HTML native converter (first document-shaped format).

## Next

- 2026-09-22: Apple Pages -> DOCX native converter (flagship). Reverse-engineering notes will live in `DOCS/formats/pages.md`.

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

- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning with `std::arch`. The word-at-a-time scanner in `io::scan` closed the CSV -> JSON gap without intrinsics; the CSV reader alone is still slower than the `csv` crate reader (see BENCHMARKS.md), so this remains the stretch target.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Done

- 2026-09-22: Foundation released as 0.1.0: framework, CSV <-> JSON, docs, CI, release pipeline, benchmark harness with all pass lines met.

## Decisions

- 2026-09-22: Writers own a 64 KiB buffer and hand sinks whole chunks. Per-field writes through `dyn Write` cost more than parsing did; buffering inside the writer was the single largest win in the first performance spike.
- 2026-09-22: Byte scanning is done eight bytes at a time with plain integer bit tricks before reaching for SIMD intrinsics. It is portable, dependency-free, and readable, and it was enough to beat the reference pipeline.
- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build.
