# Sublime

Universal efficient file conversion. One binary, zero runtime dependencies.
Apple Pages, Word, Markdown, HTML, plain text, CSV, and JSON today; every
path, its fidelity, and a map are in `DOCS/FORMATS.md`.

    sublime convert input.csv output.json
    sublime check csv json
    sublime formats
    sublime paths

## Install

Download the archive for your platform from the
[Releases](https://github.com/Chimi6/Sublime/releases) page, verify it
against `SHA256SUMS`, and put the `sublime` binary on your `PATH`. Or
build from source with `cargo install --path .` (Rust 1.85 or newer).

| Archive | Platform |
|---|---|
| `x86_64-unknown-linux-musl` | Linux, Intel or AMD, static |
| `aarch64-unknown-linux-musl` | Linux, ARM, static |
| `x86_64-apple-darwin`, `aarch64-apple-darwin` | macOS Intel, Apple silicon |
| `x86_64-pc-windows-msvc` | Windows |
| `wasm` | any browser (see below) |

## Binary or browser

| | `sublime` binary | `sublime.wasm` |
|---|---|---|
| Runs | on your machine, in scripts and pipelines | on a web page, on the visitor's machine |
| Input | files, stdin, any size, streamed | bytes in memory, whole file at once |
| Reach | every format and path | every format and path |
| Reports | loss and timing per step | a status and one message |

See `DOCS/` for project state, changelog, supported formats, and contributing.

## In the browser

The same converter ships as a WebAssembly module: every format and path,
running on the visitor's machine with nothing uploaded. Each release
publishes `sublime-<version>-wasm.zip` with `sublime.wasm`, a small
`sublime.js` that loads it, and a working page. See `wasm/README.md`.

    import { Sublime } from "./sublime.js";
    const sublime = await Sublime.load("./sublime.wasm");
    const { status, bytes } = sublime.convert(input, "pages", "docx");

## License

AGPL-3.0-or-later. Free for any use; if you build on it and distribute the
result or run it as a service, share your changes under the same terms. See
`LICENSING.md` for the plain-words version and what to do if that does not
fit your situation.
