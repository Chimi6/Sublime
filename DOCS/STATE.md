# Project State

Single source of truth for where Sublime is right now. Every entry carries the
date it was written. Update this file in the same commit as the change it
describes.

## Now

- 2026-09-27: PDF output from images (`src/io/pdf`), the first half of 0.23: a page of each image at its own size, JPEGs embedded unchanged, merges of several images into one PDF on the command line. Reading PDF (a page's image, `--page`) is next (`temp/pdf-0.23-plan.md`).
- 2026-09-27: TIFF (`src/io/tiff.rs`): the first page read in the common layouts (strips and tiles, chunky and planar, none, LZW, deflate, PackBits, the predictor, 1 to 16 bits, gray, palette, RGB, CMYK, both alphas) and 8-bit deflate strips written; every `tiff-png` line passes, memory 5 to 59 MB against 53 to 886. That completes 0.22's image set.
- 2026-09-27: ICO and CUR (`src/io/ico.rs`) with the first image operation, an area-averaging downscale (`src/image/resize.rs`): icons are read to their largest entry and written with the standard sizes from one streaming downscale. `ico-png`: an icon from a large picture passes with a margin; the 256-pixel lines are a few milliseconds and flip with load, and a 256-source icon is slower than the image crate's (its fast-level PNGs make it 78% larger).
- 2026-09-27: TGA (`src/io/tga.rs`): every common layout read and top-down RLE written as Pillow writes it; bottom-up files hold their bytes and decode last row first. Every `tga-png` line passes; the flat decode is within 1%.
- 2026-09-27: Netpbm (`src/io/netpbm.rs`): every form P1 to P7 read, PBM, PGM, PPM, and PAM written (Pillow's bytes for the first three), streaming rows both ways; `ppm-png` decode passes everywhere, encode passes on the flat and stock shapes and ties on the photo.
- 2026-09-27: QOI both ways (`src/io/qoi.rs`): decoded to Pillow's pixels and written as Pillow's bytes on 60 fixtures, streaming a row at a time both ways; every `qoi-png` line passes at 4 to 5 MB against the crates' 52 to 524. Image pairs are now generated from one codec table (`src/converters/image.rs`), so every raster format reaches every other.
- 2026-09-26: WebP both ways (`src/io/webp`): lossless and lossy decode bit-exact with libwebp on 224 fixtures, lossy streaming by macroblock row at 4 MB, and a lossless encoder whose files are 4 to 9% smaller than image-webp's and faster to write than libwebp's fastest setting. Pair `webp-png`: memory passes everywhere, five speed lines fail (Blockers).
- 2026-09-26: JPEG both ways (`src/io/jpeg`): a reader for baseline and progressive files bit-exact with libjpeg-turbo on 140 Pillow-written fixtures, streaming MCU rows into the PNG and BMP writers; a JFIF baseline writer at `--quality` (85 by default, 4:2:0 below 90) streaming sixteen rows at a time; every path holds 5 to 6 MB on a 46 MB image. Map in `DOCS/formats/jpeg.md`, oracles in `tests/jpeg_suite.rs`, pair `jpeg-png` against the `image` (zune-jpeg) and `jpeg-encoder` crates with a PSNR row. Encode is at parity with the mozjpeg port on speed, smaller at higher PSNR; decode alone is at parity with zune-jpeg.
- 2026-09-26: `bmp -> png` streams as well (`read_bmp_rows` into `PngRows`): a top-down BMP is never held (4 MB peak on a 418 MB image), a bottom-up one is held once as file bytes (66 MB on a 61 MB image, was 125).
- 2026-09-26: The `png-bmp` pair takes a stock photograph when one is present (`bench/data/stock.png`, `[stock]` rows, never committed): on an 11220 by 9775 RGBA photo with Paeth rows and profile chunks, decode 458 against 413 MB/s at 5.5 MB against 424, encode 233 against 176 MB/s writing 29 MB against 44.
- 2026-09-26: `png -> bmp` streams rows into a top-down BMP (`RowSink`, `BmpRows`): no image held (5 MB peak on a 61 MB image), and every pass line of the `png-bmp` pair passes, the photo decode at 438 against the crates' 412 MB/s, the flat at 1011 against 714. The inflater takes fdeflate's loop shape (three entries per refill), the checksums run as interleaved streams and lane sums. Spikes recorded in `temp/patterns.md`: the x86-64-v2 baseline and unchecked access moved nothing; the reference is safe Rust too, and the gap was the pipeline's extra passes and 16,000 page faults, not the compiler.
- 2026-09-26: The image category opens with an 8-bit pixel hub (`src/image`), PNG both ways (`src/io/png`: every depth, color type, palette, transparency, and interlace read in pieces as the file arrives; 8-bit written with adaptive filters), and BMP both ways (`src/io/bmp`). Maps in `DOCS/formats/png.md` and `bmp.md`, oracles in `tests/png_suite.rs` (PngSuite against Pillow) and `tests/png_bmp.rs`, pair `png-bmp` against the `png` and `image` crates. Underneath: a resumable inflater, a faster CRC-32, and a deflate matcher that spends its chain budget where matches pay.
- 2026-09-25: Batch conversion on the command line: many inputs, directories, globs, parallel workers, atomic `.part` writes, dry runs, per-file failure isolation.
- 2026-09-25: The rows-to-document bridge: CSV and TSV become a Markdown table and so reach every document format; a document's first table comes out as rows. Oracles in `tests/rows_document.rs`, pair `csv-markdown` against Miller.
- 2026-09-25: Excel workbooks, one sheet at a time: a reader that turns a chosen sheet into rows (shared strings, styles for dates, the 1904 epoch, gaps and dimension) and a writer that streams rows into a one-sheet workbook; `--sheet` on `convert`. Map in `DOCS/formats/xlsx.md`, oracles in `tests/xlsx_rows.rs`, pair `xlsx-csv` against `calamine` and `rust_xlsxwriter`.
- 2026-09-25: TSV and JSON Lines, and every row path among CSV, TSV, JSON, and JSON Lines, all streamed in constant memory. Oracles in `tests/rows.rs`, pairs `tsv-json` and `jsonl-json`.
- 2026-09-25: Direct pairs between TOML, YAML, and XML (`src/converters/hub.rs`), so the planner no longer routes them through JSON; infinities and NaN survive as floats. Oracle in `tests/hub_pairs.rs`.
- 2026-09-25: The value hub is an arena tree (`src/value/tree.rs`): the three hub formats cost about two to three bytes of memory per input byte where they cost ten, and read 1.2 to 1.9 times faster. Every pair document carries the new block.
- 2026-09-24: XML both ways through the value hub under the xmltodict mapping: a strict reader with located errors and a pretty-printing writer. Map in `DOCS/formats/xml.md`, oracles in `tests/xml_json.rs`, pair `xml-json` against `quick-xml`.
- 2026-09-24: YAML both ways through the value hub: a YAML 1.2 core-schema reader (block and flow, all scalar styles, anchors, merge keys, tags, multi-document) and a block-style writer. Map in `DOCS/formats/yaml.md`, oracles in `tests/yaml_json.rs`, pair `yaml-json` against `serde_yaml`.
- 2026-09-24: Data category, first tree format: TOML both ways through the new value hub (`src/value`: a `Value` tree, a push `ValueSink`, a `TreeBuilder`), with JSON reading into the hub. Map in `DOCS/formats/toml.md`, oracles in `tests/toml_json.rs`, pair `toml-json` passing every line against the `toml` crate.
- 2026-09-24: HTML and plain text input (phases 3 and 4): the HTML reader and the text reader emit the Markdown event stream, so a page or a text file reaches Markdown, text, Markdown JSON, and Word. Maps in `DOCS/formats/html.md`; oracles in `tests/html_document.rs`; pairs `html-markdown`, `html-text`, `html-docx`, `text-markdown`.
- 2026-09-24: Markdown into Word (phase 2): the events bridge builds the document model from the Markdown event stream, so every text input reaches Word; `markdown -> docx`, `markdown-json -> docx`. Oracles in `tests/markdown_docx.rs`, pair `markdown-docx`.
- 2026-09-24: Word input (phase 1 of the document-category plan under Next): the Word reader fills the document model and every projection out of it now takes `.docx`. Map in `DOCS/formats/docx.md`, oracles in `tests/docx_document.rs`, pairs `docx-markdown`, `docx-html`, `docx-text`.
- 2026-09-24: Sublime in the browser. `wasm/` is a second crate that exports the converter as a WebAssembly module with a small JavaScript loader (`wasm/README.md`); CI builds and smoke-tests it and every release publishes `sublime-<version>-wasm.zip`. The module is a first-class export: every library change ships in it, and its size is budgeted in `wasm/size-budget` the way the binary's is.
- 2026-09-24: Apple Pages, the flagship. The package reader, the lossless `pages-json` form, the document reader into the document model (`src/document`), the Word writer, and the Markdown event projection are in: `pages -> docx` matches Apple's own export paragraph for paragraph on 23 of 28 fixtures, and Pages reaches Markdown, HTML, text, and Markdown JSON through the model. Released as 0.6.0, with the performance work as 0.6.1 and the WebAssembly module as 0.7.0. Map in `DOCS/formats/pages.md`, fixtures in `tests/fixtures/pages/`.
- 2026-09-23: Format roadmap in `DOCS/ROADMAP.md`: every tentative format by category with a status and a priority tier (S to D, mixing value, difficulty, and novelty), plus the keystones (inflate, XML, ZIP, PNG, protobuf) that unlock whole categories.

## Next

The document category's one-way streets are closed (0.8.0 to 0.10.0) and the data category's planned set is in (0.11.0 to 0.16.0). What follows, in order:

- 2026-09-25, spreadsheets as a family (the XLSX reader and writer are the model; every entry below reads into rows and writes from them so it reaches CSV, TSV, JSON, JSON Lines, and the hub formats through the planner):
  - **Apple Numbers** (`.numbers`, Mac session): IWA package like Pages, so the Snappy and protobuf readers and the Pages fixtures pipeline apply; tables of the first sheet to rows first, then sheet and table selection through `--sheet`. Needs Numbers-made fixtures with Numbers' own CSV exports as references, which only the Mac can produce. Reader before writer; a writer needs the same object-graph work as the Pages writer and waits on that decision.
  - **OpenDocument spreadsheet** (`.ods`): the same keystones as XLSX (ZIP, XML) with `content.xml` rows and `table:table-cell` repeats; LibreOffice's own CSV exports as references. Reader and writer, a week.
  - **Google Sheets** is not a file format: it lives in Google's cloud and exports as XLSX, CSV, or ODS, which we read. Nothing to build; the docs should say so.
  - **Spreadsheet to document** (done 2026-09-25): rows become a Markdown table and reach every document format; a document's first table comes back as rows. Still open: a whole workbook as one JSON object of sheets (or one CSV per sheet) instead of one sheet per run, and a document's every table (not only the first) when the target can hold them.
  - **Excel-made fixtures** for the XLSX reader (Mac session, Excel or Numbers export), and number formats beyond dates.
- 2026-09-26, the image category after PNG, BMP, and JPEG: GIF once there is somewhere for its frames to go (APNG or video); until then its first frame is a small reader on the hub. Every codec lands in the pixel hub and ships with a pair against a real crate. JPEG levers in Tech Debt: subsampling chosen from the chroma's detail, optimized Huffman tables, progressive output, Exif orientation on the image path.
- 2026-09-25, the cheap hub batch: INI, plist (XML and binary), MessagePack, CBOR, each a day on the value tree.
- Then the document category: ODT and EPUB, which reuse the Word and HTML work almost entirely; RTF; then PDF write as its own plan.
- The Pages writer decision stays with the Mac session.

## Future

- 2026-09-22: External tier plumbing and ffmpeg wrapping for media.
- 2026-09-22: Type inference option for CSV -> JSON (opt-in, declared lossy).
- 2026-09-22: Pretty JSON output option.
- 2026-09-22: Delimiter options for CSV on the command line (semicolon, pipe); TSV shipped as its own format on 2026-09-25.

## On Hold

- (none)

## Blockers

- 2026-09-24: The Pages document paths fail the shared Pages goals of 50 MB/s of input and 64 MB peak (`DOCS/benchmarks/pages-docx.md`, `pages-markdown.md`, `pages-html.md`, `pages-text.md`). After the typed decode of the attribute tables and the interned Word styles (on top of the slim model, the streamed Word body, and the reachable decode): memory passes on every document row (28 to 46 MB), Word passes throughput under the standard's compressed-output measure (150 MB/s of input plus uncompressed output), and the text paths fail throughput at 32 to 40 MB/s against 50. A real resume converts in 3 ms at 4.8 MB, inside both goals. What remains on the dense shape, in-process: package decode 10 ms (7.5 of it Snappy), document build 25 ms, the text writers 11 ms; wall clock adds about 10 ms of process start, file I/O, and first-touch page faults (13,000 pages). The text paths are 5 to 8 ms from the line; levers under Spikes. Causes measured: a 224-byte run struct with cloned font and language strings (48 MB of model for 170,000 runs), the Word body held whole before Deflate (30 MB), and the package decoding 570 objects to use 70. Fix for 0.6.1: intern strings in the style table and slim the run, stream the Word body through the compressor, decode objects on lookup.
- 2026-09-24: `pages -> pages-json` misses the 50 MB/s goal at 28 and 40 MB/s of package bytes (`DOCS/benchmarks/pages-json.md`); the reverse direction passes at 330 MB/s. Levers: the typed decode of the attribute tables written as JSON directly (the Spikes entry below), then `Tree::decode` itself and the JSON writer's per-field work.

## Tech Debt

### Optimization

Speed and size targets we want to beat or widen, each with its measured
gap, what was tried, and the levers left. Numbers come from the pair
documents in `DOCS/benchmarks`; measured-best patterns and failed spikes
live in `temp/patterns.md`. Size wins over speed where they trade (the
WebP color cache stays).

- WebP lossless photo encode (`webp-png`): 249 against image-webp's 428 MB/s, by design (the color cache and predictor search make real photos 8 to 37% smaller; image-webp does neither). Tried: byte-wise predictors, recorded first-pass decisions, locals in both passes, four candidates, select in blocks (slower), branchless bit writer (slower). Levers: predictor scoring is 53 ms of the photo's encode, half of it select; score and compute residuals per 16-row band as rows arrive (the image is re-read five times today); threads for scoring and residuals on the command line (not wasm).
- WebP lossless size against libwebp's default effort (6% smaller than ours, thirty times the time): meta prefix codes per region, the cross-color transform, general LZ77 distances, the entropy predictor choice at the default effort if it gets cheaper.
- Lossy WebP output is not written; JPEG covers lossy output for now.
- Deflate (PNG, DOCX, XLSX): the default level now matches or beats zlib 6's ratio at about its speed. libdeflate reaches the same ratio at about twice zlib's speed: its hash-table layout and match finder are the model. Photo-shaped data costs about 11 ns a position in the chain walk (dependent loads); noise now skips seven searches in eight. A near-optimal parse would suit a smallest-output level.
- PNG writer: filters by smallest residual sum; fixed Sub compressed 1.3% smaller on a decoded lossy photo under zlib. A real-photo corpus would settle a Sub bias. The `png -> bmp` photo line's margin is thin in the harness (ours 102 against 109 ms direct).
- JPEG encoder size: optimized Huffman tables (3 to 5%), trellis quantization (5 to 10% at equal PSNR), and 4:2:0 or 4:4:4 chosen from the chroma planes' edge energy (fixed 4:2:0 costs 24 dB on hard-edged graphics). All but the last hold the coefficients.
- Markdown dense parse: level with `pulldown-cmark`, not ahead; the arena-shrinking spike under Spikes is the lever.
- `qoi-png` thin margins: the photo decode (65.3 against 64.7 MB/s, the PNG write of a noise photo is most of it) and the stock encode (583.8 against 582.0, a tie). Levers: the deflate noise path (shared with the WebP lossless decode line), and the PNG reader's unfilter on large RGBA images.
- `ppm-png` photo encode: a tie (86 to 89 ms against 88 to 90 direct; the harness read 834 against 888 MB/s). The time is the PNG decode, the inflater level with the png crate's fdeflate; the lever is the inflater's decode structure. CRC-32 over the IDAT chunks is 7% of it.
- `tga-png` flat decode: 468.4 against 466.1 MB/s, within noise. Lever: the two passes over held RLE could be one, recording row starts while decoding into the last rows' slots.
- `qoi-png` stock encode: a tie (754 against 750 to 753 ms direct). The time is the PNG decode of an RGBA photo whose rows are 77% Paeth; spiked 2026-09-27: the Paeth unfilter as an `[i16; 4]` pixel (1.6x slower) and as 16-bit lanes of one word (8% slower); the scalar form stays. The QOI encode side is at parity with the qoi crate.
- `ico-png` 256-pixel lines: milliseconds, flipping with load; the icon from a 256 source takes 8.4 ms in process against the crate's 5 for its whole run, the seven PNG encodes at our default level (44% smaller icons). Levers: the deflate matcher on dense repeats (zlib 6 takes as long on those bytes; libdeflate's structure), or a faster level for entries under 64. Spiked 2026-09-27: a nice length of 64 makes the dense-repeat deflate a third faster at 0.3% of size, still short of the crate's whole run at its fast level (its icon 78% larger).
- `xlsx -> csv` memory: 425 MB against calamine's 593 (the sheet part is inflated whole); a windowed inflate feeding the XML reader would make it constant.

- 2026-09-26: JPEG leftovers:
  - Not read: CMYK and YCCK (Adobe four-component files, common from print workflows), 12-bit samples, arithmetic coding, lossless and hierarchical modes, DNL-supplied heights. CMYK is the one users will hit.
  - The reader takes the file whole (its entropy-coded data is one bit stream); a windowed reader would make input memory constant too. The file is a tenth of the pixels, so this is small.
  - Exif orientation is reported, not applied; the transform belongs to the image path with the other whole-image operations.
  - The writer has no progressive output (it holds the coefficients, an image-path feature).
  - Encoder size levers moved to Optimization above.
  - Margins (2026-09-26): the photo encode leads jpeg-encoder by 7 to 9% since chroma is computed from summed RGB. Tried and reverted for the decode: libjpeg's color lookup tables (6% slower) and planar color groups (1 to 3% slower); a constant-chroma upsampling skip helps only specially shaped images and was not written. The flat decode's lead rests on the PNG writer.
- 2026-09-26: Image leftovers:
  - The hub is 8-bit: 16-bit PNG samples lose their low byte (reported as a loss). A 16-bit hub is the change if a lossless 16-bit path is ever wanted.
  - No metadata is carried (gamma, ICC, text, physical size); APNG frames are not read; BMP RLE is refused.
  - The PNG writer's filter heuristic (smallest sum of residual magnitudes, the standard) chooses Average on the synthetic photo where fixed Sub compressed 20% smaller under the first generator's mirrored noise; with independent noise both give the same size. An entropy estimate chose the same. A real-photo corpus would settle whether a bias toward Sub is worth it.
  - Deflate and PNG filter levers moved to Optimization above (lazy matching and priced matches shipped 2026-09-27).
- 2026-09-25: Excel leftovers:
  - The reader inflates the sheet part whole before parsing (415 MB peak on a 39 MB workbook whose sheet inflates to 326 MB); a windowed inflate feeding the XML reader would make it constant. The writer already streams.
  - One sheet per run; a whole-workbook form (a JSON object of sheets, or one CSV per sheet) is not offered.
  - Number formats other than dates are dropped; merged cells read as their top-left value.
  - No Excel-made fixtures yet; the set is hand-built from the specification.
- 2026-09-25: Value hub leftovers:
  - The TOML and YAML readers hold the input text whole beside the tree (60 to 190 MB of their peaks on the benchmark shapes); the XML reader's sliding window is the shape to port if a large config case ever matters.
  - `ValueSink` has one implementor (`TreeSink`) until a format streams.
  - TOML datetimes are validated for shape and range, not the calendar.
- 2026-09-24: WebAssembly leftovers:
  - Multi-hop paths hold each intermediate whole in memory (no threads in the browser); a single-hop path streams as on the command line. Fine for documents, a concern only for large data files through two hops.
  - The module has no size tooling beyond `opt-level = "z"`; `wasm-opt` would take 10 to 20% more off but is a toolchain dependency the build does not assume.
  - No progress or event reporting across the boundary; the host gets a status, the bytes, and one message.
- 2026-09-24: Pages document reader and Word writer leftovers:
  - Equations are written as MathML text; Office Math (OMML) from MathML is the proper output.
  - Floating objects anchor to the first paragraph on their page by counting explicit page breaks; pages that begin by overflow are not known without layout.
  - PDF and other non-picture media are left out of the Word file; converting PDF vector images to PNG needs a rasterizer.
  - Text box fills are solid colors only; gradients, image fills, strokes, and shape geometry other than rectangles are dropped. Lines and charts are dropped.
  - Comments (`table_highlight`) are not read.
  - Drop caps come out as an ordinary first letter (Apple splits them into a framed paragraph).
  - Header and footer areas (left, center, right) are joined as paragraphs; Word has no three-area header.
  - The release binary grew to 1.36 MB with the document pipeline (budget raised to 1.4 MB); std's backtrace symbolizer (gimli, addr2line, about 100 KB) is the largest non-feature cost and the lever if size matters.
- 2026-09-23: Pages leftovers from the package reader:
  - 31 registry types without a schema; they decode raw. Resolve as the document reader needs them.
  - The schema comes from community protos of two vintages; fields Pages 12 added since decode raw. Coverage is measured by the round-trip test, not by name.
  - No peer reference for the Pages pairs: the harness scales fixtures into large documents and measures against the decompression floor and our own package round trip (`DOCS/benchmarks/pages-json.md`).
- 2026-09-23: Markdown leftovers, deliberately deferred in favor of the next flagship:
  - Text -> Markdown (paragraphs only, conditional). Do it when a second text-shaped input can share it.
  - Unicode general-category tables for exact delimiter-run classification; today an approximation that no corpus example reaches (`DOCS/formats/markdown.md`, known deviations). Costs binary size; do it when a real document hits it.
  - Label case folding covers a subset of Unicode; same trigger.
  - Vendor extensions (math, wiki links, description lists, front matter, heading attributes, superscript, emoji): each needs its own corpus; add one when a path needs it.
  - Dense-input parse speed is level with `pulldown-cmark`, not ahead; the arena-shrinking spike under Spikes is the known lever.
- 2026-09-22: Review minors deferred from the foundation build, none blocking:
  - CLI: (done 2026-09-25) every output is written to a `.part` file and renamed into place. Argument-parse errors are always human-formatted even with `--log-format json`. A stdout flush error is reported ahead of the command's own error. Markdown tables from `paths --markdown` do not escape `|`.
  - Planner: strict mode runs both Dijkstra passes even when the lossless one succeeds; a `--via` plus `--strict` failure names the waypoint as the destination in the error.
  - Execution: a converter thread panic is reported as `Unsupported`; `PipeReader::read` blocks on a zero-length buffer; partial output can reach stdout before a mid-chain error is reported (inherent to streaming, needs a doc note on `execute`).
  - JSON tokenizer: `skip_value` does not check bracket type agreement (`[1}` skips); a high surrogate followed by a non-escape consumes one extra byte before erroring.
  - JSON -> CSV: truncated input is reported as `Unsupported` rather than `Malformed`; duplicate keys within one object can mask differing-key detection.
  - CSV reader: bare CR accepted as a line ending but undocumented; error paths drop the record buffer's capacity.
  - Tests: no coverage for `Progress` events, `\b`/`\f` JSON escapes, `describe_path` with mixed multi-hop fidelity, or `bytes_consumed` with a BOM present.
- 2026-09-22: JSON -> CSV from stdin buffers the whole input to memory (two passes need a rewind). Files stream in constant memory. See Spikes.
- 2026-09-22: Multi-hop chains replay events after completion instead of streaming them live.

## Spikes

- 2026-09-24: Pages performance levers past the three phases, in order of expected return (numbers are the dense benchmark shape, in-process, from `DOCS/benchmarks/pages-docx.md`):
  - Done 2026-09-24: **typed decode of the attribute tables** (`Tree::deferred`, parsed by the reader): dense-shape memory 70 -> 48 MB (passes), text paths 27 -> 32 MB/s. **Run formatting interned into Word character styles**: XML 19.3 -> 15.6 MB, Word 17 -> 20 MB/s; the gain was smaller than projected because most runs carry only a named style or a toggle, which cannot move into a style.
  - Tried 2026-09-24 and kept for correctness, without measurable gain: a one-entry cache in front of the style hash lookups (removed), 16-byte table spans, bulk overlapping copies in the Snappy decoder. The text paths sit at 58 ms wall on the dense shape against 50: package decode 10 ms (7.5 Snappy), document build 25 ms, writer 11 ms, process and page faults about 10 ms. What remains is structural: a Snappy decoder inner loop at 1.5 GB/s (about 3 ms), a document without per-paragraph vectors (one run vector per document; fewer pages touched, cheaper drop), and a model-free streaming text path (rejected so far, see below).
  - **Merging adjacent runs with equal effective formatting.** Pages splits runs at style-object boundaries that often resolve to the same look; fewer runs means less of everything downstream.
  - **Dropping the object trees once the model is built**, for the document paths only; the media bytes are already copied out.
  - **Rendering Word straight from the storage tables without a model** was considered and set aside: it saves the model build (30 ms) at the cost of a second reader inside the writer and the loss of the hub architecture every other output depends on. Revisit only if the levers above leave the goal out of reach.
  - With the first two levers the text paths (`pages -> markdown`, `html`, `text`) project to about 45 ms on the dense shape (the 50 MB/s goal is 50 ms), Word to about 65 ms, and `pages -> pages-json` to about 40 MB/s dense and past the goal on prose; the goals stay as they are. Every Pages pair document's Conclusions names the rows it fails and the levers that apply to it.

- 2026-09-23: Smaller Markdown arenas (`u32` offsets in `Line`, boxed fence data in `Kind`) to cut first-touch page faults, which are now the largest single cost in the block parser on large inputs.
- 2026-09-23: Word-at-a-time scanning for the inline parser's special characters; the byte loop with a lookup table is its largest remaining cost.
- 2026-09-23: Unicode general-category tables for exact delimiter-run classification (see `DOCS/formats/markdown.md`, known deviations).
- 2026-09-22: Single-pass JSON -> CSV with header inference from the first N objects, for the stdin case.
- 2026-09-22: `opt-level = "z"` versus `3`: measure size and speed.
- 2026-09-22: SIMD byte scanning with `std::arch`. The word-at-a-time scanner in `io::scan` closed the CSV -> JSON gap without intrinsics; the CSV reader alone is still slower than the `csv` crate reader (see benchmarks/csv-json.md), so this remains the stretch target.
- 2026-09-22: Content sniffing beyond magic bytes for extensionless input.

## Done

- 2026-09-27: The image set released as 0.22.0: QOI, Netpbm (PBM, PGM, PPM, PAM), TGA, ICO and CUR, and TIFF, every image format reaching every other through one codec table, with the first image operation (an area-averaging downscale) behind icons. The PNG reader relies on chunk CRCs (7 to 20% faster) and reads highly compressed single chunks it refused since 0.19.0; deflate stops searching on noise (the lossless WebP photo decode 53 -> 93 MB/s). Every pair's memory line passes by 2 to 100 times; the decisions above cover the two lines that trade speed for size.
- 2026-09-27: WebP released as 0.21.0 with the lazy deflate and `--quality` effort. Lossless and lossy decode bit-exact with libwebp on 224 fixtures; memory passes everywhere; both lossy decode lines, both flat decodes, and the flat encode pass. Two lines fail and the owner shipped them: the lossless photo encode (the color cache and predictor search, 8 to 37% smaller real photos) and the lossless photo decode by 2% (its PNG 7% smaller than the reference's). Every PNG, DOCX, and XLSX written is smaller and faster; `xlsx -> csv` is back to 415 MB.
- 2026-09-26: JPEG encode margins released as 0.20.1: 4:2:0 chroma from summed RGB takes the photo encode from a tie to a 7 to 9% lead over jpeg-encoder at equal or higher PSNR; every jpeg-png line passes.
- 2026-09-26: JPEG released as 0.20.0: baseline and progressive decode bit-exact with libjpeg-turbo, a streaming encoder with `--quality`, four streaming paths at a tenth of the crates' memory. The `png -> jpeg` photo line is a tie with jpeg-encoder (440 against 445 MB/s in the recorded block, 163 against 164 ms direct) at a smaller file and higher PSNR; the owner shipped it as a tie. The margin levers (Huffman coding loop, color intake, flat-chroma upsampling skip) are bundled with chroma-based subsampling under Tech Debt.
- 2026-09-26: The image category released as 0.19.0: the pixel hub, PNG and BMP both ways streaming rows in both directions, the resumable inflater, the faster CRC-32 and deflate matcher, every `png-bmp` line passing against the png and image crates on the generated shapes and a stock photograph.
- 2026-09-26: Batch conversion released as 0.18.0.
- 2026-09-25: The rows-to-document bridge released as 0.17.0, with the Markdown pairs re-validated.
- 2026-09-25: Excel workbooks one sheet at a time released as 0.16.0.
- 2026-09-25: TSV and JSON Lines with every row path released as 0.15.0.
- 2026-09-25: Direct TOML, YAML, and XML pairs released as 0.14.0.
- 2026-09-25: Value hub as an arena tree released as 0.13.1: memory 2.7 to 3.4 times lower and throughput 1.2 to 1.9 times higher on every hub pair.
- 2026-09-24: XML both ways released as 0.13.0: `xml -> json`, `json -> xml`, the `xml-json` pair passing every line against `quick-xml`; the reader streams its input.
- 2026-09-24: YAML both ways released as 0.12.0: `yaml -> json`, `json -> yaml`, the `yaml-json` pair passing every line against `serde_yaml`; the TOML and YAML writers stream.
- 2026-09-24: TOML both ways and the value hub released as 0.11.0: `toml -> json`, `json -> toml`, the `toml-json` pair passing every line against the `toml` crate.
- 2026-09-24: HTML and plain text input released as 0.10.0 (document-category phases 3 and 4): the tag-soup HTML reader and the text reader into the event stream; `html -> markdown`, `text`, `markdown-json`, `docx` and `text -> markdown`, `html`, `docx`; four benchmark pairs.
- 2026-09-24: Markdown into Word released as 0.9.0 (document-category phase 2): the events bridge with named styles, `markdown -> docx` and `markdown-json -> docx`, corpus oracles, the `markdown-docx` pair.
- 2026-09-24: Word input released as 0.8.0 (document-category phase 1): the Word reader into the document model, `docx -> markdown`, `html`, `text`, `markdown-json`, oracles against Apple's exports and our own writer, three benchmark pairs passing every line.
- 2026-09-24: WebAssembly module released as 0.7.0: `wasm/` crate, `sublime.js` loader, demo page, node smoke test, CI job with a size budget, and `sublime-<version>-wasm.zip` in every release.
- 2026-09-24: Pages performance released as 0.6.1: slim document model, streamed Word body, reachable decode, typed attribute tables, interned Word styles, reader cursors; benchmark pairs and documents for every Pages path with the compressed-output standard.
- 2026-09-24: Pages document paths released as 0.6.0: `pages -> docx` matching Apple's export on 23 of 28 fixtures, and `pages -> markdown`, `html`, `text`, `markdown-json` through the document model.
- 2026-09-23: Markdown finished as 0.3.0: Markdown writer (round-trips every specification example), Markdown -> plain text, Markdown <-> events as JSON, all faster and leaner than the reference pipelines.
- 2026-09-23: Markdown -> HTML released as 0.2.0: full CommonMark plus GFM extensions and footnotes, all specification examples passing, faster and leaner than `pulldown-cmark` on the benchmark input.
- 2026-09-22: Foundation released as 0.1.0: framework, CSV <-> JSON, docs, CI, release pipeline, benchmark harness with all pass lines met.

## Decisions

- 2026-09-27: 0.22.0 ships with two lines the owner counts as wins for size: the `ico-png` icon from a 256-pixel source takes about 8 ms in process against the image crate's 5 for its whole run, because our seven PNG entries are at our default deflate level and its are at its fast level (our icon 44% smaller); and the `qoi-png` stock encode is a tie (754 against 750 to 753 ms, the PNG decode at parity). Rule: somewhat slower but significantly smaller, still lossless, is a win when it is written down; the speed levers stay under Tech Debt > Optimization.
- 2026-09-27: WebP ships with the lossless photo encode line failing by design: the owner keeps the color cache and the predictor search for smaller files over a speed tie with image-webp, which writes neither. The lossless photo decode's 2% miss ships with it.
- 2026-09-23: Licensed AGPL-3.0-or-later, contributions inbound under Apache-2.0, no CLA, no public commercial track. The goal is that nobody paywalls the work without sharing back; exceptions are handled privately on request (`LICENSING.md`). Releases 0.1.0 to 0.3.0 stay Apache-2.0.
- 2026-09-23: The Markdown parser pushes events into a sink (`parse_into` and `EventSink`) as the production path; the iterator `Parser` stays for renderers that want to pull. Buffering events between parser and renderer cost more than rendering them.
- 2026-09-23: Object trees are arenas: one entry vector and one byte buffer per stream, chains linked by index, schema fields resolved by index. A heap node per field cost a third of the conversion time; the arena is also the layout the document reader walks.
- 2026-09-23: Pages is read into the exact object graph first (`pages-json`, lossless, byte-for-byte re-encodable), and every document-level reader is a projection over that graph. The round trip is the correctness oracle for the package layer, so schema gaps can never lose data silently.
- 2026-09-23: Reverse-engineered schemas are compiled in as packed tables generated from the community's proto files, never as a runtime dependency; the generator records its sources.
- 2026-09-22: Writers own a 64 KiB buffer and hand sinks whole chunks. Per-field writes through `dyn Write` cost more than parsing did; buffering inside the writer was the single largest win in the first performance spike.
- 2026-09-22: Byte scanning is done eight bytes at a time with plain integer bit tricks before reaching for SIMD intrinsics. It is portable, dependency-free, and readable, and it was enough to beat the reference pipeline.
- 2026-09-22: Zero runtime dependencies. Every proposed runtime dependency needs an entry in `DEPENDENCIES.md` with measured cost.
- 2026-09-22: Converters register through one explicit list in `src/registry.rs`. No link-time or build-script auto-registration.
- 2026-09-22: The library never prints; it emits typed events to an explicitly passed sink. No global logger.
- 2026-09-22: The planner picks the cheapest path by fidelity and prints it. `--strict` refuses lossy paths.
- 2026-09-22: Three tiers: Native (our code), Library (a crate compiled in), External (an installed binary). Only ffmpeg is anticipated in External.
- 2026-09-22: Benchmarks live in a standalone `bench/` crate excluded from the workspace so `csv` and `serde_json` never touch the main build. They run by hand on cause (a converter hot-path change, a release), never on a schedule, and every run is written up in `DOCS/benchmarks/<pair>.md` like a methods section.
