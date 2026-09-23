# Changelog

All notable changes to Sublime. Newest first. Follows the Keep a Changelog
layout. Every push adds a line under Unreleased; cutting a release moves that
section under a version heading.

## [Unreleased]

### Added

- (nothing yet)

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
