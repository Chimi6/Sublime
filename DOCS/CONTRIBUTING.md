# Contributing

## Principles

- Zero runtime dependencies. See `DEPENDENCIES.md` before proposing one.
- The library never prints. It emits events through the `Context` it is given.
- Full transparency on loss. Declared fidelity says what may be lost; the
  conversion report says what was.
- Prefer our own implementation. Benchmark it against the conventional crate.

## Code style

- One action per line. Prefer a named intermediate variable over a nested call.
- Descriptive names. No single-letter names outside tiny closures.
- No `unsafe` (forbidden in `Cargo.toml`). No `unwrap()` or `expect()` outside tests.
- `cargo fmt`, `cargo clippy --all-targets -- -D warnings`, and `cargo test`
  must pass before every commit.

## Where format knowledge lives

Every format's reading and writing code lives in `src/io/<format>/` and
knows nothing about conversion: a reader or parser yields records, tokens,
or events; a writer renders them. Converters in `src/converters/` only
compose those pieces. When a second converter needs the same format, it
reuses the module rather than parsing again. `io::csv`, `io::json`,
`io::markdown`, and `io::html` all follow this shape.

When a format has a published specification with conformance examples,
commit the examples under `tests/fixtures/<format>/` and run them as a test;
the specification is the reviewer.

## Adding a format or converter

Six places, and the compiler or a test catches anything you miss.

1. `src/format/formats.rs`: add a `pub static` for the format if it is new.
   Give it an id, display name, extensions, magic bytes if the format has a
   reliable signature, and a category (data, document, image, audio, video,
   archive). Categories only group listings and docs; code stays flat.
2. `src/converters/<from>_to_<to>.rs`: a struct implementing `Converter`.
   Declare `name`, `from`, `to`, `fidelity`, `tier`, and write `convert`.
   Stream through the `Input` and the `Write` you are given. Call
   `context.loss(...)` for every loss, `context.warning(...)` for tolerated
   oddities, `context.progress(...)` every few thousand records.
3. `src/converters/mod.rs`: one `pub mod` line.
4. `src/registry.rs`: one entry in `CONVERTERS` and bump the array length.
5. `tests/fixtures/<format>/`: real sample files, including nasty ones.
6. `cargo run --release -- paths --markdown > DOCS/FORMATS.md` and commit it.

Then add a line to `CHANGELOG.md` and, if the work changes direction, to
`STATE.md`.

## Fidelity contract

- `Lossless`: the output carries everything the input did for every valid input.
- `Conditional(text)`: lossless for some inputs, lossy for others; `text`
  says when. Every actual loss must emit `LossDetected`.
- `Lossy(text)`: always loses something; `text` says what.

The planner prefers lossless edges (cost 1) over conditional (10) over lossy
(100). Ties break on fewer hops, then tier (native, library, external), then
name.

## Tiers

- `Native`: our parser and writer.
- `Library`: a Rust crate compiled in. Needs a `DEPENDENCIES.md` entry.
- `External`: shells out to an installed program. Only ffmpeg is anticipated.

## Events

Converters emit `Event` values through `Context`. Renderers in `src/cli/render`
turn them into text. JSON field names in `json_lines.rs` are a stable contract
for other programs; change them only with a major version bump.

## Exit codes

| Code | Meaning |
|---|---|
| 0 | success, lossless |
| 1 | error |
| 2 | completed with loss, or `check` found a lossy path |
| 3 | no path |
| 4 | bad usage |

## Development tools

`cargo build --features dev-tools` adds `sublime inspect <package>`, which
dumps an iWork package's object graph with schema-named fields; the release
binary never includes it. The Pages schema and type tables are generated:
`scripts/gen-pages-schema.py` fetches the community proto files into
`temp/iwork-protos/` and writes `src/io/pages/{schema,types}.rs`. Regenerate
rather than edit them.

## Tests

- Unit tests live next to the code.
- `tests/cli.rs` runs the binary as a subprocess.
- Planner tests use fake converters; they never depend on the real registry.
- Round-trip tests must be byte-identical for flat, well-formed inputs.
- Fixtures under `tests/fixtures/` are stored byte-exact; `.gitattributes`
  disables line-ending normalization for them.

## Benchmarks

`bench/` is a standalone crate, run by hand, never in CI. `bench/run.sh
<pair>` generates inputs, runs ours against the reference pipelines, and
prints a table. Each pair has a document in `DOCS/benchmarks/` written like a
methods section; record every run there with the commit hash and machine, and
add a block whenever a `perf:` commit touches a converter or before a release.
See `DOCS/benchmarks/README.md` for the template and `bench/README.md` for the
harness layout.

## Licensing of contributions

By opening a pull request you license your contribution under the Apache
License, Version 2.0, and confirm you have the right to do so. You keep your
copyright. Sublime as a whole is distributed under the AGPL-3.0-or-later
(`LICENSE`, `LICENSING.md`); the permissive inbound license is what lets the
maintainer grant exceptions or other terms for the whole codebase without
asking every contributor again. The pull request template has the line to
confirm.

## Commits

Conventional commits: `feat:`, `fix:`, `docs:`, `ci:`, `test:`, `chore:`.
No trailers. `CHANGELOG.md` and, when relevant, `STATE.md` change in the same
commit as the code.

## Docs

- `DOCS/` is committed project documentation.
- `temp/` is gitignored working space for plans, specs, and notes.
- `DOCS/FORMATS.md` is generated. CI fails if it is stale.
- `DOCS/formats/<name>.md` holds reverse-engineering notes for a format.

## Releasing

0. Dry-run the build matrix first: `gh workflow run release.yml --ref <branch>`,
   and confirm all five targets succeed before tagging.
1. Move the `## [Unreleased]` entries in `CHANGELOG.md` under a new
   `## [x.y.z] - YYYY-MM-DD` heading and leave an empty Unreleased section.
2. Set `version` in `Cargo.toml` to `x.y.z`. Commit as `chore: release x.y.z`.
3. Tag and push: `git tag vx.y.z && git push origin vx.y.z`.
4. The release workflow verifies the tag, builds five targets, and publishes
   a GitHub release with archives, `SHA256SUMS`, and the changelog section.
