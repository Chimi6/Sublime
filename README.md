# Sublime

Universal efficient file conversion. One binary, zero runtime dependencies.

    sublime convert input.csv output.json
    sublime check csv json
    sublime formats
    sublime paths

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
