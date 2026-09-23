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

- 2026-09-22: Benchmark gate failed (`bench/run.sh`, commit 4ceb6e9, 10,000,000 rows / ~1 GB CSV). Two rows FAIL against DOCS/BENCHMARKS.md pass lines:
  - CSV -> JSON throughput: 165.7 MB/s ours vs 316.0 MB/s reference (`csv` + streaming `serde_json`). Ours is ~1.9x slower; needs to be >= reference.
  - Startup to first byte on a 1 KB file: 1.527 ms vs < 1 ms target. Off by 0.527 ms (~53% over budget).
  - All other rows (JSON -> CSV throughput, peak RSS in both directions and from stdin, musl binary size) PASS. See DOCS/BENCHMARKS.md, Results, 2026-09-22.
  - Next step is a spike (see Spikes: SIMD byte scanning, opt-level tuning) decided with the user, not a silent tweak, before Task 21 proceeds.

## Tech Debt

- 2026-09-22: JSON -> CSV from stdin buffers the whole input to memory (two passes need a rewind). Files stream in constant memory. See Spikes.
- 2026-09-22: Multi-hop chains replay events after completion instead of streaming them live.

## Spikes

- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning in the CSV and JSON tokenizers using `std::arch` with a scalar fallback. Stretch target: beat the `csv` crate's reader in isolation.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Decisions

- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build.
