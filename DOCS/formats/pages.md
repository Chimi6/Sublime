# Apple Pages

The flagship. Pages files are shared constantly and nothing outside Apple
reads them well. This document is the living map of the format as Sublime
understands it, tied to the fixtures that prove each claim. It grows with
the reader; `STATE.md` records what is being built now.

## Status

- `pages` <-> `pages-json`: shipped, lossless at the object level. Every
  stream of every fixture decodes into schema-named trees and re-encodes to
  its exact bytes; the JSON reads back to the same trees (CI,
  `tests/pages_fixtures.rs`).
- Reading the document as a document (text, styles, lists, tables, images,
  footnotes) into the event stream: next. The map below is what that reader
  is built from.
- Writing Pages from other formats: later, by rewriting a real Pages
  document's storage objects rather than generating Apple's object graph
  from nothing.

## Fixtures

`tests/fixtures/pages/`: twenty-seven documents, one per feature area, saved
by Pages 12 on macOS from generated Word sources (plus one authored through
the scripting dictionary), each with Apple's own Word, PDF, and text
exports as references. The fixture README lists what each contains and what
Pages drops on import.

## Package

A `.pages` file is a ZIP (every entry stored, none deflated, in the
fixtures) with:

- `Index/*.iwa`: the document as IWA streams. `Document.iwa` holds the
  document root and the body text; `ViewState.iwa`, `CalculationEngine.iwa`,
  `AnnotationAuthorStorage.iwa`, `DocumentStylesheet.iwa`,
  `ThemeStylesheet.iwa`, `Metadata.iwa`, and `Tables/*.iwa` (table tiles and
  data lists) hold the rest.
- `Data/*`: media as files (images, preset fills). Objects refer to them by
  data reference.
- `Metadata/*`: package metadata as property lists.
- `preview.jpg` and `preview-micro.jpg`: thumbnails.

Older Pages saved a directory bundle rather than a single ZIP; the reader
takes the ZIP form only (Pages converts with File > Advanced > Change File
Type).

## IWA streams

An `.iwa` is a sequence of chunks: one byte of type (always 0), a 24-bit
little-endian length, then that many bytes of one raw Snappy block. Not the
Snappy framing format: no stream identifier, no checksums. The decompressed
chunks concatenate into a protobuf stream of objects, each a varint length,
a `TSP.ArchiveInfo` (the object identifier and one `MessageInfo` per
message: type id, version, length, object and data references), then the
messages. Most objects carry one message; a few carry two (102 across the
fixtures), which the reader keeps as a list.

Message type ids are the apps' registries, which overlap between Pages,
Numbers, and Keynote in the 10000 range; Sublime's table is the Pages one.
The fixtures contain 96 distinct types. Schemas are the community's
reverse-engineered protobuf definitions (see `scripts/gen-pages-schema.py`
for sources); 658 messages reachable from the registry are compiled in as a
packed table, 31 registry names have no schema and decode raw, and any
field a schema does not know is kept by number with its wire type. Object
10016 (one per section, referring to the guide storage) is not in any
registry and stays unnamed.

## The JSON form

`pages-json` is the package as JSON: every entry in order, streams as
objects with their header and messages as schema-named trees, files as
base64. Details and the field encoding rules are in
`src/io/pages/json.rs`. It is the form the map is read from and the form
other tools can consume; `jq` over it answers most questions about a
document. Reading it back rebuilds the package; the ZIP is stored and the
streams are chunked as literal Snappy blocks, so a rebuilt file is larger
than Pages' own but identical in content.

## What the objects mean

Established from the fixtures with `sublime inspect` (a `dev-tools` build).

- **Document root.** `TP.DocumentArchive` (10000) refers to the stylesheet,
  the floating drawables, the body storage, the section, the theme, and the
  settings.
- **Text.** `TSWP.StorageArchive` (2001): `text` is one string for the whole
  storage with `\n` between paragraphs; attribute tables map character
  offsets to objects: `table_para_style` (paragraph style per paragraph
  start), `table_char_style` (character style runs), `table_list_style`,
  `table_para_data` (list levels), `table_attachment` (inline drawables,
  footnote marks), `table_smartfield` (hyperlinks, bookmarks, fields),
  `table_footnote`, `table_section`, `table_insertion` and `table_deletion`
  (tracked changes), `table_highlight` (comments). The body storage in
  `text-styles` shows the whole document's text in one string with eleven
  paragraph style runs.
- **Styles.** `TSWP.ParagraphStyleArchive` (2022) and
  `TSWP.CharacterStyleArchive` (2021) carry property archives and inherit
  through `super.parent`; `TSS.StylesheetArchive` maps names to styles.
  Headings are paragraph styles by name (`Heading 1` and so on), which is
  how the reader will recognize them.
- **Sections and page templates.** `TP.SectionArchive` (10011) refers to
  `TP.SectionTemplateArchive` (10143; the registries still call it
  `PageMasterArchive`), which holds three header and three footer storages
  (first, left, right pages).
- **Lists.** `TSWP.ListStyleArchive` (2023) per list style; the level of a
  paragraph is in `table_para_data`.
- **Tables.** `TST.TableInfoArchive` and `TST.TableModelArchive` with cell
  data in `Tables/Tile*.iwa` and strings in `Tables/DataList*.iwa`. Not yet
  mapped beyond decoding.
- **Drawables.** `TSD.*` archives for images, shapes, lines, and text boxes;
  `TP.FloatingDrawablesArchive` and `TP.DrawablesZOrderArchive` place them.
  Not yet mapped beyond decoding.

## Known deviations

- Rebuilt packages use stored ZIP entries and uncompressed Snappy blocks:
  bigger files, same content. A Snappy compressor is on the tech debt list.
- 31 registry types have no schema and one type has no name; both decode
  raw and survive a round trip.

## Sources

- IWA and the object stream: obriensp's iWorkFileFormat documentation and
  SheetJS's notes.
- Schemas: numbers-parser (shared packages, recent) and orcastor's
  iwork-converter (Pages package, recent), both derived from the app
  binaries' embedded descriptors.
- Registries: dunhamsteve's iwork (Pages and common tables) and
  numbers-parser's mapping.
