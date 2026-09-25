# Format Roadmap

A working list of every file type Sublime tentatively intends to support,
by category, with what exists today checked off and a priority tier for the
rest. The tier list at the bottom is the reading order. This document is
opinionated and will change; `STATE.md` records what is actually being
built right now.

## How to read this

- **Status.** `[x]` shipped, `[~]` partly shipped (one direction, or a
  subset), `[ ]` planned.
- **Tier.** A mix of value, difficulty, and novelty, not any one of them:
  - **S** — do next. Very valuable, or the flagship that makes people talk.
  - **A** — high value for moderate effort, or easy and useful.
  - **B** — useful and reasonable, or novel and easy.
  - **C** — niche, or hard for the value it returns; still wanted.
  - **D** — someday. Very hard, very niche, or better left to an external
    tool behind the External tier.
- **Effort.** Rough size of the format code, assuming zero dependencies:
  `S` a day, `M` a week, `L` a month, `XL` a project.
- **Directions.** Which conversions are meant, and their fidelity. A
  format listed without a partner is read into or written from a common
  model (events for documents, rows for tables, pixels for images, samples
  for audio) and so reaches every other format in its category.

Fidelity follows `CONTRIBUTING.md`: lossless, conditional (lossless for
inputs of a certain shape), or lossy, always declared and always shown.

## Keystones

Zero runtime dependencies means every codec and container is our own code.
A handful of building blocks unlock whole rows of the tables below, so they
are priorities in their own right and are listed here rather than as
formats.

| Keystone | Unlocks | Effort |
|---|---|---|
| Inflate and deflate (zlib, gzip, zip) | ZIP, gzip, PNG, DOCX, XLSX, PPTX, ODT, EPUB, KMZ, WOFF, NPZ, JAR, Minecraft region, many game archives | M (inflate shipped) |
| XML parser and writer (streaming) | DOCX, ODT, SVG, KML, GPX, plist, FB2, X3D, TTML, RSS, XLSX, 3MF, DAE | M |
| ZIP container | Everything that is "a zip of XML": Office, OpenDocument, EPUB, KMZ, ORA, 3MF, Krita | S (reader and stored writer shipped) |
| Protobuf wire decoder, Snappy | Pages, Numbers, Keynote (IWA), OSM PBF | M (shipped; Snappy decode only) |
| PNG codec | The image hub: every image format converts through pixels and out to PNG first | M (on top of inflate) |
| JPEG baseline codec | Photos in and out; JPEG in DOCX and PDF; camera raw previews | L |
| Compound File Binary (OLE2) | Legacy `.doc`, `.xls`, `.ppt`, Outlook `.msg` | M |
| RIFF and ISO base media containers | WAV, AVI, WebP; MP4, MOV, M4A, HEIF boxes | S each |
| EBML | MKV, WebM | S |
| ASN.1 DER | Certificates, keys, PKCS structures | M |
| LZMA and bzip2 decoders | XZ, 7z, TAR.XZ, TAR.BZ2, CHD | M each |
| Zstandard decoder | `.zst`, Arrow, Parquet with zstd | M |
| CFB-free PDF object model | PDF text extraction and PDF writing | L |
| TrueType table reader | Fonts, PDF embedding, SVG glyph export | M |

## Data

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| CSV | csv | [x] | — | — | Shipped: CSV <-> JSON. |
| JSON | json | [x] | — | — | Shipped. Hub for all data formats. |
| TSV and delimited variants | tsv, psv | [ ] | A | S | Same reader with a delimiter option; also semicolon CSV (European Excel). |
| JSON Lines | jsonl, ndjson | [ ] | A | S | Streaming rows; natural partner for CSV. |
| YAML | yaml, yml | [x] | — | — | Shipped both ways (0.12.0): YAML 1.2 core schema, anchors, merge keys, multi-document, through the value hub. Notes in `formats/yaml.md`. |
| TOML | toml | [x] | — | — | Shipped both ways (0.11.0): TOML <-> JSON through the value hub. Notes in `formats/toml.md`. |
| XML | xml | [x] | — | — | Shipped both ways (0.13.0): XML <-> JSON under the xmltodict mapping, declared conditional. Notes in `formats/xml.md`. |
| INI | ini, cfg | [ ] | A | S | Trivial reader; INI <-> JSON/TOML. |
| Java properties | properties | [ ] | B | S | Escapes and continuations; properties <-> JSON. |
| .env | env | [ ] | B | S | Shell-quoted key values. |
| JSON5, HJSON | json5, hjson | [ ] | B | S | Comments and trailing commas; one-way to JSON, or JSON pretty variants out. |
| MessagePack | msgpack | [ ] | A | S | Binary JSON; exact JSON round trip. |
| CBOR | cbor | [ ] | A | S | Same shape as MessagePack; used in COSE, WebAuthn. |
| BSON | bson | [ ] | B | S | MongoDB dumps; BSON <-> JSON with extended JSON for dates and ids. |
| UBJSON, Smile | ubj | [ ] | D | S | Rare. |
| Apple plist | plist (XML and binary) | [ ] | A | S | macOS everywhere; binary bplist <-> XML plist <-> JSON is a real daily need. |
| SQLite | sqlite, db | [ ] | S | L | Read the database file format directly: tables to CSV/JSON, one file per table or a chosen table. Enormous value; nobody expects a converter to do it. Writing SQLite from CSV is a second, harder step. |
| dBase | dbf | [ ] | B | S | Ancient, still everywhere (shapefiles, legacy exports). DBF <-> CSV. |
| Excel workbook | xlsx | [ ] | S | M | Needs ZIP, inflate, XML. Sheets to CSV/JSON; CSV to XLSX. Shared strings, dates as serials, formulas as values. |
| Excel legacy | xls | [ ] | C | L | BIFF8 over OLE2. Read only. |
| OpenDocument spreadsheet | ods | [ ] | A | M | Same keystones as XLSX. |
| Apple Numbers | numbers | [ ] | A | L | Shares the IWA keystone with Pages. Tables to CSV. |
| Parquet | parquet | [ ] | A | L | Thrift-compact metadata, plain and dictionary encodings, snappy and zstd. Read to CSV/JSON first; write later. High value for data people. |
| Apache Arrow IPC / Feather | arrow, feather | [ ] | B | M | Flatbuffers metadata plus raw buffers. Read first. |
| Avro | avro | [ ] | B | M | Schema in the container; deflate or snappy blocks. |
| ORC | orc | [ ] | D | L | Rare outside Hadoop. |
| NumPy | npy, npz | [ ] | A | S | `npy` is a header plus raw array: trivial and loved. `npz` needs ZIP. To CSV/JSON. |
| MATLAB | mat | [ ] | B | M | v5 format is documented; v7.3 is HDF5 (see below). |
| HDF5 | h5, hdf5 | [ ] | C | XL | Huge spec; a reader for the common subset (contiguous and chunked datasets, gzip filter) is feasible. |
| NetCDF | nc | [ ] | C | M | Classic format is small; NetCDF-4 is HDF5. |
| R data | rds, RData | [ ] | B | M | gzip plus XDR serialization; data frames to CSV. |
| SPSS, Stata, SAS | sav, dta, sas7bdat | [ ] | C | M each | Statistical files; DTA is documented and simple, SAV moderate, SAS7BDAT reverse-engineered. |
| Protocol Buffers text and binary | pb, txtpb | [ ] | C | M | Binary without a schema decodes to a generic tree (wire types only); with a `.proto` it is a compiler project. |
| ASN.1 DER, PEM | der, pem, crt, cer | [ ] | A | M | PEM <-> DER is base64 with armor (trivial). DER to JSON (a readable dump of a certificate) needs the ASN.1 keystone. No cryptography. |
| JWK, JWT | jwk, jwt | [ ] | B | S | Decode to JSON; no signing or verification. |
| BibTeX, RIS, EndNote | bib, ris, enw | [ ] | B | S | Citation formats; academics convert these constantly. To and from JSON and each other. |
| vCard | vcf | [ ] | A | S | Contacts to CSV/JSON and back. |
| iCalendar | ics | [ ] | A | S | Events to CSV/JSON; also CSV to ICS for bulk imports. |
| Chess PGN and FEN | pgn | [ ] | B | S | Games to JSON; novelty with a real audience. |
| GraphQL, OpenAPI | graphql, yaml | [ ] | D | — | Schema languages, not data; out of scope unless as YAML/JSON. |

## Document

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| Markdown | md, markdown | [x] | — | — | Shipped: CommonMark plus GFM, event stream, and a writer back to Markdown. Hub for text documents. |
| HTML | html, htm | [x] | — | — | Shipped both ways: written from the event stream, and read into it by a tag-soup-tolerant reader (0.10.0): HTML -> Markdown, text, Markdown JSON, Word. Notes in `formats/html.md`. |
| Plain text | txt | [x] | — | — | Shipped both ways: text out of every document path, and text in as paragraphs (0.10.0): text -> Markdown, HTML, Word. |
| Markdown as JSON | markdown-json | [x] | — | — | Shipped both ways: the event stream as JSON, lossless round trip. |
| Apple Pages | pages | [~] | S | XL | Package reader, lossless `pages-json`, document reader, Word writer, and Markdown, HTML, and text paths shipped (0.6.0); performance to the benchmark pass lines next. The flagship. ZIP of Snappy-framed protobuf (IWA); schemas are reverse-engineered and published. Pages -> DOCX and Pages -> Markdown/HTML. Nobody outside Apple does this well, and Pages files are shared constantly. Notes will live in `formats/pages.md`. |
| Apple Keynote | key | [ ] | B | L | Same keystone as Pages; to PPTX or a Markdown outline. |
| Word | docx | [~] | S | L | ZIP plus XML (WordprocessingML). Written from the model (Pages -> Word, 0.6.0, Markdown -> Word, 0.9.0) and read into it (Word -> Markdown, HTML, text, 0.8.0); HTML and text into Word next through the same bridge. Notes in `formats/docx.md`. |
| Word legacy | doc | [ ] | C | L | Word 97 binary over OLE2; text and basic formatting extraction only. |
| Rich Text Format | rtf | [ ] | A | M | Text format with a documented grammar; RTF <-> Markdown/HTML/DOCX. Still emitted by many systems. |
| OpenDocument text | odt | [ ] | A | M | ZIP plus XML; close to DOCX in shape. |
| PDF | pdf | [ ] | S | XL | Two different jobs. Writing PDF from Markdown/HTML/DOCX (layout engine, fonts, images) is the most requested output of any converter. Reading PDF for text extraction is moderate; full PDF -> DOCX is a research project and stays lossy. |
| EPUB | epub | [ ] | A | M | ZIP of XHTML plus manifest. Markdown/HTML/DOCX -> EPUB is a favorite of writers; EPUB -> Markdown. |
| Kindle MOBI, AZW3 | mobi, azw3 | [ ] | B | M | PalmDOC and HUFF/CDIC compression, documented by the community; read to HTML/EPUB. Writing MOBI is dead technology; AZW3 (KF8) write is possible. |
| FictionBook | fb2 | [ ] | B | S | XML; popular in Eastern Europe. |
| DjVu | djvu | [ ] | D | XL | Custom wavelet and JB2 codecs. |
| CHM | chm | [ ] | D | L | LZX compression plus ITSS container. |
| reStructuredText | rst | [ ] | A | M | Python world's Markdown; rst <-> Markdown through the events. |
| AsciiDoc | adoc | [ ] | A | M | Technical writing; a real grammar, larger than Markdown. |
| Org-mode | org | [ ] | B | M | Emacs; devoted users. |
| Textile, Creole, MediaWiki | textile, wiki | [ ] | C | M | Wiki markups; MediaWiki -> Markdown has real demand for migrations. |
| LaTeX | tex | [ ] | B | M | Markdown -> LaTeX out is easy and valuable; LaTeX in is a subset (sections, lists, emphasis, math passthrough). |
| troff / man pages | 1, man, roff | [ ] | B | S | Markdown -> man is small and a delight for CLI authors. |
| Jupyter notebook | ipynb | [ ] | A | S | JSON; to Markdown (cells and outputs), to plain script, and back. |
| R Markdown, Quarto | Rmd, qmd | [ ] | B | S | Markdown with fenced metadata. |
| WordPerfect | wpd | [ ] | D | L | Obscure and binary; skip unless asked. |
| Apple TextEdit RTFD | rtfd | [ ] | C | S | RTF plus attachments in a bundle. |
| Email | eml, mbox | [ ] | A | S | RFC 5322 parsing; EML <-> MBOX, and to Markdown/HTML/JSON. |
| Outlook message | msg | [ ] | B | M | OLE2 keystone; to EML. |
| Subtitles | srt, vtt, ass, ssa, sbv, ttml, lrc | [ ] | A | S | All small text grammars; any to any. High value per hour of work. |

## Image

The image hub is a pixel buffer plus metadata. Every decoder lands there;
every encoder leaves from there. PNG is the first target because it is
lossless and universal.

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| PNG | png | [ ] | S | M | Keystone. Inflate, filters, palettes, 16-bit, APNG frames. |
| JPEG | jpg, jpeg | [ ] | S | L | Baseline and progressive decode; baseline encode with quality. Exif passthrough. |
| GIF | gif | [ ] | A | S | LZW both ways; animation frames. |
| BMP | bmp | [ ] | A | S | Trivial; still asked for. |
| TIFF | tif, tiff | [ ] | A | M | Strips and tiles, common compressions (none, PackBits, LZW, deflate). Also the base of DNG and GeoTIFF. |
| WebP | webp | [ ] | B | L | VP8 lossy decode is a real codec; lossless WebP is its own codec. Encode is XL. Consider External. |
| AVIF, HEIC, JPEG XL | avif, heic, jxl | [ ] | D | XL | AV1, HEVC, and JXL codecs: External tier. |
| Netpbm | ppm, pgm, pbm, pam | [ ] | A | S | Trivial and beloved by tooling. |
| farbfeld, QOI | ff, qoi | [ ] | B | S | Tiny formats; QOI is a day's work and a crowd-pleaser. |
| TGA | tga | [ ] | B | S | Games and textures; RLE. |
| PCX | pcx | [ ] | C | S | DOS-era; RLE. |
| ICO, CUR, ICNS | ico, cur, icns | [ ] | A | S | Icon containers (PNG and BMP inside); PNG <-> ICO is a constant developer need. |
| SVG | svg | [ ] | B | XL | Rasterizing SVG is a renderer (paths, strokes, text, filters). A subset rasterizer (paths, basic shapes, fills, strokes) is L and covers most icons. SVG -> PNG only. |
| PSD | psd | [ ] | B | M | Composite image extraction is straightforward; layers to PNGs is a step more. |
| GIMP XCF | xcf | [ ] | C | M | Layers and tiles; composite export. |
| OpenRaster, Krita | ora, kra | [ ] | C | S | ZIP of PNGs plus XML. |
| Aseprite | ase, aseprite | [ ] | B | M | Documented, popular with pixel artists; frames to PNG/GIF. |
| Camera raw | dng, cr2, nef, arw | [ ] | C | XL | DNG is TIFF plus demosaicing; vendor formats are reverse-engineered. Embedded JPEG preview extraction is S and worth doing early. |
| DDS, KTX, KTX2 | dds, ktx | [ ] | B | M | Texture containers; BC1 to BC7 decode, plain formats. Game modding audience. |
| Amiga IFF ILBM | iff, lbm | [ ] | C | S | Bitplanes and HAM modes; retro charm. |
| Mac PICT, MacPaint | pict, pntg | [ ] | D | M | Obscure. |
| Sun raster, SGI RGB, XPM, XBM | ras, sgi, xpm, xbm | [ ] | C | S | Simple legacy formats; cheap once the hub exists. |
| Exif and XMP metadata | — | [ ] | A | S | Not a format: read metadata to JSON, strip or carry it across conversions. |

## Audio

The audio hub is PCM samples plus channel layout and rate. Encoders for
lossy codecs are the hardest code in this document; decoders are tractable.

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| WAV | wav | [ ] | S | S | RIFF keystone; PCM, float, extensible. The hub's file form. |
| AIFF, AIFF-C | aiff, aif, aifc | [ ] | A | S | Apple's WAV. |
| AU, VOC, CAF | au, voc, caf | [ ] | C | S | Simple containers. |
| FLAC | flac | [ ] | A | M | Decoder is well documented; encoder (LPC, Rice) is M more. Lossless both ways. |
| MP3 | mp3 | [ ] | A | L | Decoder (layer III) is a classic L project; encoder is XL and patent-free now but still XL: External for encode. |
| Ogg Vorbis | ogg, oga | [ ] | B | L | Decoder feasible; encoder XL. |
| Opus | opus | [ ] | C | XL | External. |
| AAC, M4A, ALAC | aac, m4a | [ ] | C | XL | ALAC decode is M and documented; AAC is External. |
| WavPack, APE, TTA | wv, ape, tta | [ ] | D | L | Niche lossless codecs. |
| MIDI | mid, midi | [ ] | A | S | To JSON/CSV (events) and back is trivial and useful. MIDI -> WAV needs a synthesizer: a small general-MIDI sine or wavetable renderer is B and fun. |
| Tracker modules | mod, xm, s3m, it | [ ] | B | M | Rendering modules to WAV is a well-trodden path and a genuine novelty. MOD is S. |
| Chiptune | sid, nsf, spc, gbs, vgm | [ ] | C | L each | Rendering means emulating a sound chip (SID 6581, 2A03, SPC700, Game Boy APU, YM2612). VGM is a register log and the easiest (M). Great novelty, real effort. |
| N64 audio | aifc (VADPCM) | [ ] | B | M | VADPCM decode to AIFF/WAV; the codebook format is documented by the decompilation community. |
| Speech and telephony | gsm, amr | [ ] | D | L | Skip. |

## Video and subtitles

Codecs are External-tier work (ffmpeg wrapping, see `STATE.md` Future).
Containers can be handled natively.

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| Remuxing | mp4, mov, mkv, webm, avi | [ ] | B | L | Move streams between containers without decoding: MKV -> MP4 is the common ask. Needs ISO BMFF, EBML, RIFF keystones. |
| Transcoding | any | [ ] | B | S (plumbing) | External tier around ffmpeg with fidelity declared lossy; the framework already anticipates it. |
| Animated GIF <-> video | gif | [ ] | C | — | Through External once transcoding exists; GIF <-> APNG/WebP animation natively. |
| Subtitles | see Document | — | A | — | Listed under Document. |

## Archive and compression

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| ZIP | zip | [ ] | S | M | Keystone. Deflate and store; zip64; also unlocks every Office format. Conversions: ZIP <-> TAR, ZIP <-> directory. |
| gzip, zlib | gz | [ ] | S | S | On top of inflate/deflate. |
| TAR | tar | [ ] | A | S | ustar and pax headers. |
| bzip2 | bz2 | [ ] | B | M | Decoder M, encoder M. |
| XZ, LZMA | xz, lzma | [ ] | B | M | Decoder M; encoder L. |
| Zstandard | zst | [ ] | B | M | Decoder M; encoder L. Also needed for modern Parquet and Arrow. |
| 7z | 7z | [ ] | C | L | LZMA plus a container with many options. |
| RAR | rar | [ ] | D | XL | Proprietary; read only via the published unrar source, which is not license-compatible with reimplementation freedom. Skip. |
| cpio, ar | cpio, a, deb | [ ] | C | S | `.deb` is `ar` of tarballs; easy once TAR exists. |
| ISO 9660 | iso | [ ] | B | M | Read and write; Joliet and Rock Ridge. |
| CAB | cab | [ ] | C | M | MSZIP is deflate; LZX is more. |
| LHA, ARJ, ZOO | lzh, arj, zoo | [ ] | D | M | Retro; only if the retro computing category takes off. |
| StuffIt | sit, sitx | [ ] | D | L | Reverse-engineered, obscure. |

## Fonts

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| TrueType, OpenType | ttf, otf, ttc | [ ] | A | M | Table reader keystone. TTF <-> OTF is mostly relabeling; TTC split and join. |
| WOFF | woff | [ ] | A | S | TTF <-> WOFF is deflate plus a header. Web developers do this daily. |
| WOFF2 | woff2 | [ ] | B | L | Needs a Brotli decoder (and encoder for the reverse). |
| Type 1 | pfb, pfa | [ ] | C | S | PFB <-> PFA is trivial; Type 1 -> OTF is M. |
| Bitmap fonts | bdf, pcf, fon | [ ] | C | S | BDF <-> PCF, and to PNG sheets. |
| Glyphs to SVG | — | [ ] | B | M | Export outlines as SVG paths; fun and useful for designers. |

## 3D and models

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| STL | stl | [ ] | A | S | ASCII <-> binary is trivial and constantly needed by 3D-printing users. |
| OBJ | obj, mtl | [ ] | A | S | Text; the hub for meshes. |
| PLY | ply | [ ] | B | S | ASCII and binary. |
| glTF, GLB | gltf, glb | [ ] | A | M | JSON <-> binary packing is S; mesh conversion to and from OBJ is M. The modern standard. |
| 3MF, AMF | 3mf, amf | [ ] | B | S | ZIP plus XML, and XML; printing formats. |
| OFF, X3D, VRML | off, x3d, wrl | [ ] | C | S | Academic and legacy. |
| COLLADA | dae | [ ] | C | M | XML; big schema. |
| FBX | fbx | [ ] | D | L | Proprietary binary; reverse-engineered. Skip for now. |
| Quake models | mdl, md2, md3 | [ ] | B | S | Documented, small, and a joy: MD2 -> OBJ/glTF. |
| Source engine SMD | smd | [ ] | C | S | Text format; note the extension clash with Sega SMD (detection by content). |
| Minecraft schematics | schematic, litematic, nbt | [ ] | B | S | NBT keystone (see Games). |

## Geospatial

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| GeoJSON | geojson | [ ] | A | S | JSON; the hub. |
| GPX | gpx | [ ] | A | S | XML; tracks and waypoints; GPX <-> GeoJSON <-> CSV/KML. Runners and hikers convert these all the time. |
| KML, KMZ | kml, kmz | [ ] | A | S | XML (KMZ is ZIP). |
| Shapefile | shp, shx, dbf | [ ] | B | M | Multi-file; DBF keystone. To GeoJSON. |
| WKT, WKB | wkt, wkb | [ ] | B | S | Geometry text and binary. |
| TopoJSON | topojson | [ ] | C | M | Topology encoding; to GeoJSON. |
| OSM XML, PBF | osm, pbf | [ ] | C | M | Protobuf keystone; huge files, streaming. |
| GeoTIFF | tif | [ ] | C | M | TIFF plus tags; to PNG with a sidecar. |

## Scientific and medical

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| FASTA, FASTQ | fasta, fa, fastq | [ ] | A | S | Bio text formats; to CSV/JSON, FASTQ -> FASTA. Huge audience. |
| GenBank, GFF, BED, VCF | gb, gff, bed, vcf | [ ] | B | S | Note the `vcf` clash with vCard: detect by content. |
| SAM, BAM | sam, bam | [ ] | C | M | BAM is BGZF (gzip blocks). |
| PDB, mmCIF | pdb, cif | [ ] | B | S | Protein structures; to JSON/CSV. |
| FITS | fits | [ ] | B | M | Astronomy: headers to JSON, image HDUs to PNG. Novel and real. |
| DICOM | dcm | [ ] | A | M | Medical images: tags to JSON, pixel data to PNG. High value, well documented. |
| NIfTI | nii | [ ] | C | S | Neuroimaging volumes; slices to PNG. |
| EDF, BDF | edf | [ ] | C | S | EEG signals; to CSV. |
| Intel HEX, S-record | hex, srec | [ ] | A | S | Firmware images <-> raw binary; embedded developers need this weekly and reach for `objcopy`. |
| ELF, PE, Mach-O | elf, exe, dylib | [ ] | D | — | Not conversions; out of scope beyond a header dump. |

## Games and consoles

Where the fun lives. Most of these are tiny, documented by preservation
communities, and have no good cross-platform tool.

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| N64 ROM byte order | z64, v64, n64 | [ ] | A | S | The same ROM in native big-endian (`z64`), 16-bit byte-swapped (`v64`, Doctor V64 dumps), and little-endian (`n64`). Pure reordering, lossless; detect the real order from the header rather than the extension. |
| N64 saves | eep, sra, fla, mpk, srm | [ ] | A | S | EEPROM, SRAM, FlashRAM, Controller Pak; `srm` is the RetroArch bundle of all four. SRAM and FlashRAM differ by 32-bit byte swap between PC and Wii emulators. Small, lossless, and a perennial forum question. |
| N64 textures | — | [ ] | C | M | CI4/CI8/RGBA16/IA formats to PNG given dimensions and palette; needs a ROM-hacking style interface. |
| N64 audio | aifc | [ ] | B | M | See Audio (VADPCM). |
| N64 models | — | [ ] | D | L | Display lists (F3DEX); a romhacking project, not a converter. |
| SNES ROM header | smc, sfc | [ ] | A | S | `smc` often carries a 512-byte copier header; `sfc` is clean. Detect by size mod 1024 and header bytes; strip or add. |
| Sega Genesis ROM | smd, bin, md, gen | [ ] | A | S | `smd` is interleaved in 16 KB blocks (even bytes then odd bytes); `bin` is linear. De-interleave and interleave. |
| NES ROM headers | nes, unf | [ ] | B | S | iNES, NES 2.0, and UNIF; header fix-ups and conversion. |
| Game Boy, GBA saves | sav, srm, sgm | [ ] | B | S | Size padding and emulator wrapper differences. |
| Save state formats | st, pj, etc. | [ ] | D | — | Emulator-internal and unstable; skip. |
| PlayStation disc images | bin/cue, iso, ecm, chd | [ ] | B | M | ECM <-> BIN (error-code stripping) is S and classic; BIN/CUE <-> ISO for single-track data discs is S; CHD needs zlib and LZMA (M). |
| Minecraft NBT | nbt, dat, mca | [ ] | A | S | NBT <-> JSON is easy and hugely popular (SNBT too); region files need inflate. |
| Doom WAD | wad | [ ] | B | S | Lumps out (graphics to PNG with palette, sounds to WAV, music MUS -> MIDI); a much-loved format with a simple layout. |
| Quake PAK, PK3 | pak, pk3 | [ ] | B | S | PAK is a trivial archive; PK3 is ZIP. |
| PICO-8 cartridge | p8, p8.png | [ ] | B | S | Cart data steganographically stored in a PNG; extract Lua and assets, and rebuild. Pure novelty, small, delightful. |
| Amiga, C64, Spectrum, Atari, Apple II images | adf, d64, t64, prg, tap, tzx, atr, dsk, woz | [ ] | C | S–M | Disk and tape images; T64 -> PRG and TAP <-> TZX are S; filesystem extraction (ADF, D64, DSK) is M each. A coherent retro computing set has real preservation-community appeal. |
| Pokémon saves | sav | [ ] | D | M | Checksums and generation-specific layouts; fan tools cover it. |

## Notebooks, config, and everything else

| Format | Extensions | Status | Tier | Effort | Directions and notes |
|---|---|---|---|---|---|
| Base64, hex, uuencode | — | [ ] | B | S | Encodings as pseudo-formats: `--to base64`. Cheap and used. |
| Hex dump | hex, txt | [ ] | C | S | Binary to and from `xxd` style dumps. |
| Diagrams (DOT, Mermaid, PlantUML) | dot, mmd, puml | [ ] | D | XL | Rendering needs a layout engine. Text-to-text between them is C. |
| Mind maps, outlines | opml, mm | [ ] | C | S | OPML <-> Markdown outlines; FreeMind XML. |
| Feeds | rss, atom | [ ] | B | S | XML to JSON/Markdown. |
| Log formats | syslog, clf, json | [ ] | C | S | To CSV/JSON; open-ended. |
| Spreadsheets to Markdown tables | — | [ ] | A | S | CSV/XLSX -> Markdown table and back; small and constantly wanted. |

## Tier list

The board. Within a tier the order is loose. Shipped formats are not
listed.

| Tier | Formats |
|---|---|
| **S** | Apple Pages (flagship) · SQLite read · XLSX · DOCX · PDF write · PNG (keystone) · JPEG · WAV · ZIP and gzip (keystones) · HTML read (on pause) |
| **A** | TSV and JSON Lines · INI · MessagePack and CBOR · plist · ODS and ODT · Numbers · Parquet read · NumPy · PEM/DER · vCard and iCalendar · RTF · EPUB · reStructuredText · AsciiDoc · Jupyter · Email (EML, MBOX) · Subtitles (SRT, VTT, ASS, TTML) · GIF · BMP · TIFF · Netpbm · ICO/ICNS · Exif and XMP · AIFF · FLAC · MP3 decode · MIDI · TAR · TrueType/OpenType · WOFF · STL · OBJ · glTF · GeoJSON · GPX · KML · FASTA/FASTQ · DICOM · Intel HEX and S-record · N64 ROM byte order · N64 saves · SNES headers · Genesis SMD/BIN · Minecraft NBT · Spreadsheet to Markdown table |
| **B** | Java properties · .env · JSON5 · BSON · dBase · Arrow · Avro · MATLAB · R data · JWK/JWT · BibTeX/RIS · PGN · Keynote · MOBI/AZW3 · FictionBook · Org-mode · LaTeX · man pages · R Markdown · Outlook MSG · WebP decode · farbfeld and QOI · TGA · SVG subset rasterizer · PSD · Aseprite · DDS/KTX · Ogg Vorbis decode · Tracker modules · N64 audio · Video remuxing · Transcoding via External · bzip2 · XZ · Zstandard · ISO 9660 · WOFF2 · Glyphs to SVG · PLY · 3MF/AMF · Quake models · Minecraft schematics · Shapefile · WKT/WKB · GenBank/GFF/BED/VCF · PDB · FITS · NES headers · Game Boy saves · PlayStation images · Doom WAD · Quake PAK · PICO-8 · Base64 and hex · Feeds |
| **C** | XLS · HDF5 · NetCDF · SPSS/Stata/SAS · Protobuf generic · Word `.doc` · Textile/MediaWiki · RTFD · PCX · XCF · ORA/Krita · Camera raw · IFF ILBM · legacy rasters · AU/VOC/CAF · Opus, AAC, ALAC · Chiptune (SID, NSF, SPC, GBS, VGM) · Animated GIF via External · 7z · cpio/ar/deb · CAB · Type 1 fonts · Bitmap fonts · OFF/X3D/VRML · COLLADA · Source SMD · TopoJSON · OSM · GeoTIFF · SAM/BAM · NIfTI · EDF · N64 textures · Retro disk images · Hex dumps · OPML · Log formats · Diagram text-to-text |
| **D** | UBJSON · ORC · GraphQL/OpenAPI · DjVu · CHM · WordPerfect · AVIF/HEIC/JXL · PICT · WavPack/APE · Speech codecs · RAR · LHA/ARJ/ZOO · StuffIt · FBX · ELF/PE · Save states · Pokémon saves · N64 models · Diagram rendering |

## Sequencing notes

- Keystones first, in the order that unlocks the most: inflate/deflate and
  ZIP, then XML, then PNG. Those three make XLSX, DOCX, ODT, EPUB, KMZ, and
  every image path possible. Protobuf and Snappy come with Pages.
- Games and consoles ship as a batch: the N64, SNES, and Genesis
  conversions together are a week of work with disproportionate reach in
  the communities that will notice a new converter.
- Small text formats (subtitles, vCard, iCalendar, GPX, BibTeX, Intel HEX)
  are the cheapest way to grow the format count with things people use.
- Anything marked External waits for the External tier plumbing; it is
  listed so the plan for it is visible, not because it is near.

## Sources consulted

- N64 ROM byte orders: [N64 ROM format reference](http://n64dev.org/romformats.html), [Doctor V64](https://en.wikipedia.org/wiki/Doctor_V64), [n64romconvert](https://docs.rs/n64romconvert).
- N64 saves: [N64SaveConverter](https://github.com/Ninjiteu/N64SaveConverter), [Save converters, Emulation General Wiki](https://emulation.gametechwiki.com/index.php/Save_converters), [ra_mp64_srm_convert](https://lib.rs/crates/ra_mp64_srm_convert).
- SNES headers: [ROM file formats, SNESdev Wiki](https://snes.nesdev.org/wiki/ROM_file_formats), [super-beheader](https://github.com/aitorciki/super-beheader).
- Genesis SMD: [.smd, Sega Wiki](https://sega.fandom.com/wiki/.smd), [smd2bin](https://github.com/paulguy/smd2bin).
- Pages: [IWA, Just Solve the File Format Problem](http://fileformats.archiveteam.org/wiki/IWA), [Reverse Engineering iWork](https://andrews.substack.com/p/reverse-engineering-iwork), [python-pages](https://github.com/isoparametric/python-pages/), [pyiwa](https://github.com/ChloeTigre/pyiwa).
