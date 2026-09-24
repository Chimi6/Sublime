# Sublime in the browser

`sublime.wasm` is the whole converter as a WebAssembly module: every
format and path the command line has, running inside the visitor's
browser. Files never leave their machine. The module has no dependencies
and no JavaScript beyond the one small file that moves bytes in and out.

## Use it on a page

Put `sublime.wasm` and `sublime.js` next to your page and:

```html
<script type="module">
  import { Sublime } from "./sublime.js";
  const sublime = await Sublime.load(new URL("./sublime.wasm", import.meta.url));
  const bytes = new Uint8Array(await file.arrayBuffer()); // from an <input type="file">
  const result = sublime.convert(bytes, "pages", "docx");
  if (result.status === "converted" || result.status === "converted-with-loss") {
    download(result.bytes, "document.docx"); // a Blob and an <a download>
  }
</script>
```

`index.html` in this folder is a complete page: pick a file, choose a
target from the paths the module reports, convert, download. Copy it or
read it.

The API (`sublime.js`):

- `Sublime.load(source)`: `source` is a URL, a `Response`, or the bytes.
- `sublime.convert(bytes, from, to)` returns `{ status, bytes, message }`.
  `status` is `converted`, `converted-with-loss` (the message says what
  was dropped), `failed` (the message is the error), `no-path`, or
  `unknown-format`. `bytes` is a `Uint8Array` you own.
- `sublime.formats()`: `[{ id, name, extensions, category }]`.
- `sublime.paths()`: `[{ from, to, fidelity }]`.
- `sublime.formatFor(fileName)`: a format id from the extension, or null.

Format ids are the ones `sublime formats` prints (`pages`, `docx`,
`markdown`, `html`, `text`, `csv`, `json`, ...).

The page must be served over HTTP(S), not opened from disk, because
browsers refuse to fetch a module from `file://`. Any static host works;
no server code is involved.

## Build it

```
rustup target add wasm32-unknown-unknown
cargo build --release --target wasm32-unknown-unknown --manifest-path wasm/Cargo.toml
node wasm/smoke.mjs
```

The module lands at
`wasm/target/wasm32-unknown-unknown/release/sublime_wasm.wasm`; the
smoke test loads it, converts fixtures, and prints its size. Every
release publishes `sublime-<version>-wasm.zip` with the module, the
JavaScript, this page, and the licenses, built from the same commit as
the binaries.

## How it works

The crate (`src/lib.rs`) exports a few C-ABI functions over the module's
linear memory: `alloc`/`dealloc` for the host's buffers, `convert`, and
readers for the output and message buffers. It calls the library's
planner and converters directly; multi-step paths run one step at a time
through in-memory buffers, since the browser offers no threads to the
module. It is the one place in Sublime with `unsafe`: the lines that turn
the host's pointer and length into a slice, and the one that reclaims a
buffer, documented in place.
