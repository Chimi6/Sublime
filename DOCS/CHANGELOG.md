# Changelog

All notable changes to Sublime. Newest first. Follows the Keep a Changelog
layout. Every push adds a line under Unreleased; cutting a release moves that
section under a version heading.

## [Unreleased]

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
