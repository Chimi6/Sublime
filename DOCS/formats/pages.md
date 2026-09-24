# Apple Pages

The flagship. Pages files are shared constantly and nothing outside Apple
reads them well. This document is the living map of the format as Sublime
understands it, tied to the fixtures that prove each claim. It grows with
the reader; `STATE.md` records what is being built now.

## Status

- `pages` <-> `pages-json`: shipped, lossless at the object level. Every
  stream of every fixture decodes into schema-named trees and re-encodes to
  its exact bytes; the JSON reads back to the same trees (CI,
  `tests/pages_fixtures.rs`). Rebuilt packages are Snappy-compressed and
  come out slightly smaller than Pages' own.
- `pages` -> `docx`: shipped. The document reader (`src/io/pages/document.rs`)
  projects the object graph into the document model (`src/document`): body
  text with paragraph and character styles, direct formatting, lists,
  links, footnotes, page and section breaks, tables (merged cells, fills,
  header rows, column widths), inline and anchored images, floating text
  boxes and images, headers and footers (first, even, odd), page setup,
  columns, tables of contents, page-number fields, tracked changes, and
  equations (as MathML). The Word writer (`src/io/docx/writer.rs`) renders
  it. On 22 of the 27 fixtures the output matches Apple's own Word export
  paragraph for paragraph in text and style names (CI,
  `tests/pages_docx.rs`); the other five differ where Apple's export is the
  lossy one (drop caps split, ruby base text dropped, equations flattened
  to Office Math without operator glyphs) or where text boxes anchor to a
  different paragraph.
- Pages -> Markdown, HTML, text: next, as a projection of the document
  model into the Markdown event stream.
- Writing Pages from other formats: later, by rewriting a real Pages
  document's storage objects rather than generating Apple's object graph
  from nothing.

On the largest fixture (733 KB, 87 KB of object streams, the rest images)
the package reads to JSON in about 11 ms and rebuilds from JSON in 18 ms,
at 8 MB peak memory. A real two-page resume converts to Word in 5 ms at
7 MB peak.

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
for sources); 670 messages reachable from the registry are compiled in as a
packed table, 31 registry names have no schema and decode raw, and any
field a schema does not know is kept by number with its wire type. Type
10016 is in no registry; its payload (a repeated pair of page index and
guide storage reference) identifies it as `TP.UserDefinedGuideMapArchive`.

## The JSON form

`pages-json` is the package as JSON: every entry in order, streams as
objects with their header and messages as schema-named trees, files as
base64. Details and the field encoding rules are in
`src/io/pages/json.rs`. It is the form the map is read from and the form
other tools can consume; `jq` over it answers most questions about a
document. Reading it back rebuilds the package; the ZIP is stored and the
streams are Snappy-compressed in 64 KiB chunks, so a rebuilt file matches
Pages' own in size and is identical in content.

## What the objects mean

Established from the fixtures with `sublime inspect` (a `dev-tools` build).

- **Document root.** `TP.DocumentArchive` (10000) refers to the stylesheet,
  the floating drawables, the body storage, the section, the theme, and the
  settings.
- **Text.** `TSWP.StorageArchive` (2001): `text` is one string for the whole
  storage with `\n` between paragraphs (`\r` in documents written through
  the scripting dictionary); attribute tables map character offsets, counted
  in UTF-16 units, to objects: `table_para_style` (paragraph style per
  paragraph start; an entry without an object means the previous style
  continues), `table_char_style` (character style runs; the default style
  is named `None`), `table_list_style`, `table_para_data` (`first` is the
  list level, `second` whether the paragraph starts a list),
  `table_attachment` (inline drawables, page-number fields, TOC entries),
  `table_smartfield` (hyperlinks as `TSWP.HyperlinkFieldArchive.url_ref`,
  bookmarks), `table_footnote`, `table_section`, `table_layout_style`
  (column layouts), `table_insertion` and `table_deletion` (tracked
  changes, each entry a `TSWP.ChangeArchive` whose session names the
  author), `table_highlight` (comments).
- **Special characters.** U+2028 is a line break inside a paragraph; U+0005
  a page break, U+0004 a section break, and U+000C a layout (column)
  break, each leading the paragraph that follows the break and each an
  empty paragraph of its own in the style table; U+FFFC an attachment;
  U+000E a footnote mark. A newline at the very end of the text is an empty
  paragraph Pages does not show.
- **Styles.** `TSWP.ParagraphStyleArchive` (2022) and
  `TSWP.CharacterStyleArchive` (2021) carry property archives and inherit
  through `super.parent`; unnamed variation styles carry direct formatting
  over a named parent. Alignment 0 left, 1 right, 2 center, 3 justify,
  4 natural; the first-line indent is measured from the margin (hanging
  when it is less than the left indent); line spacing modes 0 relative,
  1 minimum, 2 exact. `TSWP.TOCEntryStyleArchive` (2026) wraps a paragraph
  style for table of contents entries (`TOC 1`, `TOC 2`, and so on).
- **Sections and page templates.** `table_section` entries start sections;
  `TP.SectionArchive` (10011) names its first, even, and odd
  `TP.SectionTemplateArchive` (10143; the registries still call it
  `PageMasterArchive`) and whether the first and even pages differ. A
  template holds three header and three footer storages (left, center,
  right areas). Page size and margins are on `TP.DocumentArchive` fields
  30 to 37 (`page_width`, `page_height`, the four margins, and the header
  and footer margins). Columns come from `table_layout_style` entries:
  `TSWP.ColumnStyleArchive` (2024) variation chains ending in
  `column_properties.columns.equal_columns.count`.
- **Fields.** `TSWP.NumberAttachmentArchive` (2043) in a header or footer
  is the page number (`super.kind` 0) or the page count (1).
- **Lists.** `TSWP.ListStyleArchive` (2023) per list style: `label_types`
  per level (2 text, 3 number), `number_types` (Roman and letter variants
  in groups of three by pattern: `%1.`, `(%1)`, `%1)`), `strings`, `indents`,
  `text_indents`.
- **Tables.** A `TSWP.DrawableAttachmentArchive` in the text refers to
  `TST.TableInfoArchive` (6000), whose `tableModel` is
  `TST.TableModelArchive` (6001): row and column counts, header row count,
  default sizes, and `base_data_store`. Column widths are
  `TST.HeaderStorageBucket` entries (`columnHeaders`), row heights the
  `rowHeaders.buckets`. Cells live in `TST.Tile` (6002) `rowInfos`:
  `cell_offsets` is one little-endian `u16` per column (`0xFFFF` empty,
  times four when `has_wide_offsets`) into `cell_storage_buffer`, where
  each record (version 5) is: version byte, cell type byte, six bytes, a
  `u32` of flags at offset 8, then the fields the flags select in order:
  `0x1` decimal128 (16 bytes), `0x2` double, `0x4` seconds since 2001,
  `0x8` string id, `0x10` rich text id, `0x20` cell style id, `0x40` text
  style id, and twelve more four-byte ids (conditional styles, formula,
  control, formula error, suggestion, and the number, currency, date,
  duration, text, and boolean formats, comment, import warning). Cell types:
  0 empty, 2 and 10 number, 3 text, 5 date, 6 boolean, 7 duration, 8 error,
  9 rich text. Ids index `TST.TableDataList` (6005) objects: the string
  table (`listType` 1, `entries.string`), the rich text table (8,
  `rich_text_payload` -> `TST.RichTextPayloadArchive.storage`, a full text
  storage), and the style table (4, `reference` to a
  `TST.CellStyleArchive` with `cell_properties.cell_fill.color` or a
  paragraph style). Merged regions are not in the table: the model's
  `merge_owner.owner_id` names a formula owner in
  `TSCE.CalculationEngineArchive.dependency_tracker.formula_owner_info`,
  whose `range_dependencies.back_dependency[].internal_range_reference.range`
  entries are the merged rectangles. Pages exports a table before the
  paragraph that held it, which then stays as an empty paragraph.
- **Images.** `TSD.ImageArchive` (3005): `data.identifier` names a
  `TSP.PackageMetadata.datas` entry whose `file_name` is the file under
  `Data/`; the size is `super.geometry.size` (points), the alt text
  `super.accessibility_description`. Inline attachments carry NaN offsets;
  anchored ones carry `h_offset_type` (2 is the page) and offsets. An
  equation is an image archive with `equation_source_text` (MathML).
- **Floating drawables.** `TP.FloatingDrawablesArchive` (10010)
  `page_groups[]` place drawables by page index: `TSWP.ShapeInfoArchive`
  (2011, a text box when it has `owned_storage`; the fill is in its
  `TSWP.ShapeStyleArchive`), `TSD.ImageArchive`, `TSD.GroupArchive` (3008,
  `children` positioned relative to the group), lines, and charts.
- **Table of contents.** `TSWP.TOCAttachmentArchive` (2241) refers to
  `TSWP.TOCInfoArchive` (2240), a shape whose `owned_storage` holds the
  rendered entries with `TSWP.TSWPTOCPageNumberAttachmentArchive` (2010)
  `page_number` strings at the tab stops.
- **Footnotes.** `TSWP.FootnoteReferenceAttachmentArchive` at the mark,
  `contained_storage` the note's text, which starts with a placeholder
  U+FFFC for the mark.

## Known deviations

- 31 registry types have no schema; they decode raw and survive a round
  trip.
- Equations reach Word as their MathML text (`E=mc2`), not as Office Math.
- Floating objects are anchored to the first paragraph on their page,
  counting only the page breaks the document spells out; on a page that
  starts by overflow they land on the paragraph after the last explicit
  break.
- Media Word cannot show as a picture (PDF, for instance) is left out of
  the Word file; only solid text box fills are kept; comments and
  highlights are not read.

## Sources

- IWA and the object stream: obriensp's iWorkFileFormat documentation and
  SheetJS's notes.
- Schemas: numbers-parser (shared packages, recent) and orcastor's
  iwork-converter (Pages package, recent), both derived from the app
  binaries' embedded descriptors.
- Registries: dunhamsteve's iwork (Pages and common tables) and
  numbers-parser's mapping.
