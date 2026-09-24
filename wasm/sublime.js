// Sublime in the browser: the JavaScript side of the module's contract.
// No bundler and no dependencies; an ES module that works in a page, a
// worker, or node.
//
//   import { Sublime } from "./sublime.js";
//   const sublime = await Sublime.load("./sublime.wasm");
//   const result = sublime.convert(bytes, "pages", "docx");
//   // result.status: "converted" | "converted-with-loss" | "failed" | "no-path" | "unknown-format"
//   // result.bytes: Uint8Array, result.message: string
//
// The module keeps its output in its own memory until the next call, so
// `result.bytes` is a copy the caller owns.

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
    const { instance } = await WebAssembly.instantiate(bytes, {});
    return new Sublime(instance);
  }

  constructor(instance) {
    this.exports = instance.exports;
  }

  /** Converts `input` (Uint8Array) from one format id to another. */
  convert(input, from, to) {
    const fromBytes = new TextEncoder().encode(from);
    const toBytes = new TextEncoder().encode(to);
    const fromPtr = this.#place(fromBytes);
    const toPtr = this.#place(toBytes);
    const inputPtr = this.#place(input);
    let code;
    try {
      code = this.exports.convert(fromPtr, fromBytes.length, toPtr, toBytes.length, inputPtr, input.length);
    } finally {
      this.exports.dealloc(inputPtr, input.length);
      this.exports.dealloc(toPtr, toBytes.length);
      this.exports.dealloc(fromPtr, fromBytes.length);
    }
    const status = STATUS[code] ?? "failed";
    const bytes = status === "converted" || status === "converted-with-loss" ? this.#output() : new Uint8Array(0);
    return { status, bytes, message: this.#message() };
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
    const ptr = this.exports.alloc(bytes.length);
    new Uint8Array(this.exports.memory.buffer, ptr, bytes.length).set(bytes);
    return ptr;
  }

  #output() {
    const ptr = this.exports.output_ptr();
    const len = this.exports.output_len();
    return new Uint8Array(this.exports.memory.buffer, ptr, len).slice();
  }

  #message() {
    const ptr = this.exports.message_ptr();
    const len = this.exports.message_len();
    return new TextDecoder().decode(new Uint8Array(this.exports.memory.buffer, ptr, len));
  }
}
