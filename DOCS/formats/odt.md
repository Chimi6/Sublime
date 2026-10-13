# OpenDocument Text (odt)

ODT is a ZIP package of XML parts: `content.xml` holds the body and the
automatic styles it uses, `styles.xml` the named styles, list styles, page
layouts, and master pages (with their headers and footers), and pictures,
formula objects, and embedded fonts are parts of their own, all named in
`META-INF/manifest.xml`. Its first entry is `mimetype`, stored, which is
how a file is recognised. Sublime reads it into the document model
(`src/io/odt/reader.rs`) and writes the model as ODT
(`src/io/odt/writer.rs`); `styles.rs` parses the properties both share. ODT
reaches and is reached from every document format: Word, Pages, RTF,
Markdown, HTML, text, and PDF (both ways).

## Status

- `odt -> docx`, `pages`, `rtf`, `markdown`, `html`, `text`, `pdf`: shipped.
- `docx`, `pages`, `rtf`, `markdown`, `html`, `text`, `pdf` `-> odt`: shipped.

Measured with `scripts/odt-check` on 89 Word documents (the repository's
fixtures and a corpus of real documents). LibreOffice renders every file,
the reference included, and pandoc is the other converter:

| Path | Matches the reference's page count | Mean word recall |
|---|---|---|
| Word -> ODT (reference: the Word file) | ours 82, pandoc 55 (LibreOffice's own: 89) | ours 0.957, pandoc 0.840 |
| LibreOffice ODT -> Word (reference: the ODT) | ours 76, pandoc 53 (LibreOffice's own: 87) | ours 0.974, pandoc 0.887 |

LibreOffice's own conversion is not a fair line: the reference is its own
rendering, so its text extraction matches by construction. Where our
recall dips the text is there and laid out differently: formulas render as
LibreOffice's formula objects rather than Word's equations, endnotes
become footnotes, and ruby and text effects are drawn apart.

The writer's `settings.xml` states the layout rules LibreOffice takes for a
Word document (`WORD_LAYOUT` in `writer.rs`): without them LibreOffice lays
out an ODT by an older version's rules. One choice, its printer-independent
metrics at low rather than high resolution, was picked on this corpus (11
of 12 page counts against 4 on the files it moved); on 23 held-out Word
files (Cocoa RTF and Markdown saved as Word by LibreOffice and pandoc) the
two are within one file of each other, and ours matches 17.

Oracles (`tests/odt_document.rs`): LibreOffice's ODT of eight fixture
sources reads to the same Markdown as the Word sources; its notes,
comment threads, tracked changes, pictures, merged cells, and page setup
read as Word's do; our own ODT of every Word source reads back to the same
Markdown and the same headers and footers, in a package whose parts are all
in the manifest and well formed; and every `--effort` writes the same
document.

## What is read and written

| Feature | ODT | Notes |
|---|---|---|
| Styles | `style:style` (paragraph, text), `style:default-style`, `style:parent-style-name` | Named styles in `styles.xml`; automatic styles in `content.xml` (and `styles.xml`'s own for headers and footers) are folded into the paragraph or run that uses them. Names escape spaces as `_20_`; the default paragraph style is `Standard`. |
| Character formatting | `style:text-properties` | Font, size, bold, italic, underline, strike, colour, highlight, position (super and subscript), caps, small caps, hidden, letter spacing, scaling, language. |
| Paragraph formatting | `style:paragraph-properties` | Alignment, indents, spacing, line height, keep-with-next, keep-together, widows and orphans, breaks, borders, background, tab stops (relative to the indent), outline level. |
| Lists | `text:list`, `text:list-style`, `text:list-level-style-number`/`-bullet` | Nested lists; restarts through `text:continue-numbering`, `text:continue-list`, and `text:start-value`; an item in another list's style through `text:style-override`. |
| Tables | `table:table`, `table:table-column`, `table:covered-table-cell` | Spans from `number-columns-spanned`/`-rows-spanned` and the covered cells; borders, shading, padding, vertical alignment, header rows, row heights, column widths, nesting. |
| Pages | `style:page-layout`, `style:master-page`, `style:master-page-name` | A master page change starts a section. ODF margins leave out the page's border and padding and the header and footer, so the model's margins add them back. Columns from `style:columns`, or a `text:section` with columns. |
| Headers and footers | `style:header`, `style:header-left`, `style:header-first` (and footers) | |
| Fields | `text:page-number`, `text:page-count`, `text:a` | Links and page fields are live; other fields keep their shown text. Form checkboxes (`field:fieldmark`) read as ☒ and ☐. |
| Notes | `text:note` | Endnotes read as footnotes (the model has no endnotes). |
| Comments | `office:annotation`, `office:annotation-end` | Replies through `loext:parent-name`. |
| Tracked changes | `text:tracked-changes`, `text:changed-region`, `text:change-start`/`-end`, `text:change` | A deletion's text, kept in its changed region, is put back where `text:change` marks it. |
| Pictures | `draw:frame` with `draw:image` | Size, alt text (`svg:title`, `svg:desc`), and anchoring; the writer keeps each picture's file name. |
| Formulas | `draw:frame` with `draw:object` | The object's `content.xml` is MathML, the model's form. |
| Text boxes | `draw:frame` with `draw:text-box` | Their paragraphs follow the anchor's; the writer writes the model's text boxes and charts (as a table in a text box). |
| Embedded fonts | `svg:font-face-src`, `Fonts/` | Read and written with `loext:font-style` and `loext:font-weight`; Word's embedded fonts (`w:embedRegular` and its kin, de-obfuscated with `w:fontKey`) carry both ways. |

## Writing

The writer writes the package LibreOffice writes: `mimetype` stored first,
`content.xml`, `styles.xml`, `meta.xml`, `settings.xml` (`EmbedFonts` when
the document has fonts), the manifest, and the pictures, formula objects,
and fonts. Paragraphs and runs share automatic styles where their
properties are equal, and sections with the same page share a master page,
the first of which is `Standard`. A page or column break inside a paragraph
splits it, since ODF breaks only before or after a paragraph. `--effort`
applies to the package's compression (`DOCS/formats/docx.md`).

## Not carried

Document metadata (the model has none; the writer's `meta.xml` names only
Sublime), text wrap around pictures, drawn shapes, charts (the writer's
table stands in), OLE objects other than formulas, and form controls beyond checkboxes. Flat ODT (`.fodt`, the
single XML file) and the ODF master and template documents are later
steps.
