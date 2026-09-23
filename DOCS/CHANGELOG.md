# Changelog

All notable changes to Sublime. Newest first. Follows the Keep a Changelog
layout. Every push adds a line under Unreleased; cutting a release moves that
section under a version heading.

## [Unreleased]

### Added

- `DOCS/ROADMAP.md`: tentative formats by category with status and priority tier, keystone building blocks, and a tier list.

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
