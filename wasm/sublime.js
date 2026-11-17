// Sublime in the browser: the JavaScript side of the module's contract.
// No bundler and no dependencies; an ES module that works in a page, a
// worker, or node.
//
//   import { Sublime } from "./sublime.js";
//   const sublime = await Sublime.load("./sublime.wasm");
//   const result = sublime.convert(bytes, "pages", "docx");
//   // result.status: "converted" | "converted-with-loss" | "failed" | "no-path" | "unknown-format"
//   // result.bytes: Uint8Array, result.message: string
//   // result.parts: files in the result; more than one means bytes is a ZIP of them
//
// The module keeps its output in its own memory until the next call, so
// `result.bytes` is a copy the caller owns.
//
// A conversion that traps (the engine's stack or memory ran out) leaves the
// module's state broken, so the instance is dropped and a fresh one made
// from the compiled module; that call returns `status: "failed"` with
// `crashed: true`, and later calls work as before.

const STATUS = ["converted", "failed", "converted-with-loss", "no-path", "unknown-format"];

export class Sublime {
  /** `source` is a URL, a Response, an ArrayBuffer, or a Uint8Array of `sublime.wasm`. */
  static async load(source) {
    let bytes;
    if (source instanceof Uint8Array || source instanceof ArrayBuffer) {
      bytes = source;
    } else {
      const response = source instanceof Response ? source : await fetch(source);
      if (!response.ok) {
        throw new Error(`could not load Sublime: ${response.status} ${response.statusText}`);
      }
      bytes = await response.arrayBuffer();
    }
    const { module, instance } = await WebAssembly.instantiate(bytes, {});
    return new Sublime(instance, module);
  }

  /** `module` (the compiled `WebAssembly.Module`) lets a trapped instance be replaced. */
  constructor(instance, module) {
    this.exports = instance.exports;
    this.module = module;
    this.restarting = null;
  }

  /** Resolves once the module can convert again after a crash. */
  async ready() {
    await this.restarting;
  }

  /**
   * Converts `input` (Uint8Array) from one format id to another. `options`
   * takes the command line's flags without their dashes:
   * `{ quality: 60, effort: "fast", sheet: "Data", page: 2, delimiter: ";" }`.
   * One the conversion does not use comes back in `warnings`.
   */
  convert(input, from, to, options = {}) {
    if (!this.exports) {
      return Sublime.#crashed("Sublime is reloading after a failed conversion; await ready() and retry");
    }
    const fromBytes = new TextEncoder().encode(from);
    const toBytes = new TextEncoder().encode(to);
    const optionBytes = new TextEncoder().encode(
      Object.entries(options).map(([key, value]) => `${key}=${value}`).join("\n"),
    );
    const fromPtr = this.#place(fromBytes);
    const toPtr = this.#place(toBytes);
    const inputPtr = this.#place(input);
    const optionsPtr = this.#place(optionBytes);
    let code;
    try {
      code = this.exports.convert_with(
        fromPtr, fromBytes.length, toPtr, toBytes.length, inputPtr, input.length, optionsPtr, optionBytes.length,
      );
    } catch (error) {
      // A trap: the instance's memory and stack are not to be trusted, so
      // nothing more is called on it (not even dealloc).
      this.#restart();
      return Sublime.#crashed(`the conversion crashed (${error.message}); Sublime was reloaded`);
    }
    this.exports.dealloc(optionsPtr, optionBytes.length);
    this.exports.dealloc(inputPtr, input.length);
    this.exports.dealloc(toPtr, toBytes.length);
    this.exports.dealloc(fromPtr, fromBytes.length);
    const status = STATUS[code] ?? "failed";
    const bytes = status === "converted" || status === "converted-with-loss" ? this.#output() : new Uint8Array(0);
    // More than one part: `bytes` is a ZIP holding a file per part.
    const parts = status === "converted" || status === "converted-with-loss" ? this.exports.output_parts() : 0;
    const warningText = new TextDecoder().decode(
      new Uint8Array(this.exports.memory.buffer, this.exports.warnings_ptr() >>> 0, this.exports.warnings_len() >>> 0),
    );
    const warnings = warningText ? warningText.split("\n") : [];
    return { status, bytes, message: this.#message(), parts, warnings, crashed: false };
  }

  static #crashed(message) {
    return { status: "failed", bytes: new Uint8Array(0), message, parts: 0, warnings: [], crashed: true };
  }

  /** A fresh instance from the compiled module: at once where the engine
   * allows it, otherwise (a large module on a browser's main thread)
   * asynchronously, with `ready()` resolving when it is done. */
  #restart() {
    this.exports = null;
    if (!this.module) {
      return;
    }
    try {
      this.exports = new WebAssembly.Instance(this.module, {}).exports;
    } catch {
      this.restarting = WebAssembly.instantiate(this.module, {}).then((instance) => {
        this.exports = instance.exports;
        this.restarting = null;
      });
    }
  }

  /** Every format: `[{id, name, extensions, category}]`. */
  formats() {
    this.exports.formats();
    return JSON.parse(this.#message());
  }

  /** Every conversion path: `[{from, to, fidelity}]`. */
  paths() {
    this.exports.paths();
    return JSON.parse(this.#message());
  }

  /** The format id for a file name, by extension, or null. */
  formatFor(fileName) {
    const extension = fileName.toLowerCase().split(".").pop();
    for (const format of this.formats()) {
      if (format.extensions.includes(extension)) {
        return format.id;
      }
    }
    return null;
  }

  #place(bytes) {
    // Addresses are unsigned; past 2 GiB the i32 the export returns is not.
    const ptr = this.exports.alloc(bytes.length) >>> 0;
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  }

  #output() {
    const ptr = this.exports.output_ptr() >>> 0;
    const len = this.exports.output_len() >>> 0;
    return new Uint8Array(this.exports.memory.buffer, ptr, len).slice();
  }

  #message() {
    const ptr = this.exports.message_ptr() >>> 0;
    const len = this.exports.message_len() >>> 0;
    return new TextDecoder().decode(new Uint8Array(this.exports.memory.buffer, ptr, len));
  }
}
