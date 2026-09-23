# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-22: Building the foundation: framework, CSV <-> JSON, docs, CI, release pipeline.

## Next

- 2026-09-22: Markdown -> HTML native converter (first document-shaped format).
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

- 2026-09-22: JSON -> CSV from stdin buffers the whole input to memory (two passes need a rewind). Files stream in constant memory. See Spikes.
- 2026-09-22: Multi-hop chains replay events after completion instead of streaming them live.

## Spikes

- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning with `std::arch`. The word-at-a-time scanner in `io::scan` closed the CSV -> JSON gap without intrinsics; the CSV reader alone is still slower than the `csv` crate reader (see BENCHMARKS.md), so this remains the stretch target.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Decisions

- 2026-09-22: Writers own a 64 KiB buffer and hand sinks whole chunks. Per-field writes through `dyn Write` cost more than parsing did; buffering inside the writer was the single largest win in the first performance spike.
- 2026-09-22: Byte scanning is done eight bytes at a time with plain integer bit tricks before reaching for SIMD intrinsics. It is portable, dependency-free, and readable, and it was enough to beat the reference pipeline.
- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build.
