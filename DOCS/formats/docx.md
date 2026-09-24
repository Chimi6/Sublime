# Word (docx)

Office Open XML WordprocessingML: a ZIP of XML parts. Sublime writes it
from the document model (`src/io/docx/writer.rs`, since 0.6.0) and reads
it into the same model (`src/io/docx/reader.rs`, since 0.8.0), so every
path out of the model is a path out of Word. This is the living map of
what the reader and writer handle, tied to the tests that prove it.

## Status

- `docx -> markdown`, `html`, `text`, `markdown-json`: shipped. The reader
  fills the document model and the Markdown projection renders it
  (`src/document/markdown.rs`).
- `pages -> docx`: shipped (see `pages.md`).
- `markdown -> docx` and `markdown-json -> docx`: shipped (0.9.0). The
  events bridge (`src/document/from_events.rs`) builds the model from the
  Markdown event stream with named styles Word users know (`Heading 1` to
  `Heading 6`, `Quote`, `Source Code`, `Source Text`, `Horizontal Line`,
  two list styles), and the Markdown projection recognizes the same names
  (and Word's and LibreOffice's: `Code`, `HTML Preformatted`, `Block Text`,
  `Intense Quote`, `HTML Code`, `Verbatim Char`), so Word documents that
  use them read back as code blocks, quotes, and rules. Oracles in
  `tests/markdown_docx.rs`: 472 CommonMark and 21 GFM examples survive the
  bridge (compared as HTML without what Word has no form for: paragraph
  tags, link titles, code languages, soft breaks), and a document with
  every construct survives the whole trip through a Word file.
- `html -> docx` and `text -> docx`: shipped (0.10.0), through the same
  bridge from the HTML and text readers (`html.md`).

Oracles (`tests/docx_document.rs`): Apple's own Word exports of the 28
Pages fixtures (`tests/fixtures/pages/reference/*.docx`) read to the same
text as the Pages documents on 22 of them (the six left out are the ones
where Apple's export itself differs, listed in the test); and our own
Word output of every fixture reads back to the same Markdown the Pages
document gives directly, `equations` excepted (the writer puts equations
in as text).

## Parts read

| Part | What is taken |
|---|---|
| `_rels/.rels` | the main part's name (falls back to `word/document.xml`) |
| `word/_rels/*.rels` | relationship ids to hyperlinks, images, headers, footers |
| `styles.xml` | document defaults (`docDefaults`), paragraph and character styles with `basedOn` chains, numbering styles |
| `numbering.xml` | abstract numberings (levels: format, text, indents, start), `numStyleLink`/`styleLink` indirection, `num` instances with level-0 start overrides |
| `settings.xml` | `evenAndOddHeaders` only |
| `footnotes.xml`, `endnotes.xml` | note bodies by id, separators skipped; both become footnotes |
| `header*.xml`, `footer*.xml` | blocks per section reference (default, first with `titlePg`, even with the setting) |
| `document.xml` | the body: paragraphs, tables, section properties |
| `media/*` | image bytes, once per part, named by file |

## Body elements

| Element | Model |
|---|---|
| `w:p`, `w:pPr` | paragraph: style, `numPr` (list item), alignment, indents (hanging as a negative first line), spacing (line rule to relative/minimum/exact), keep flags, outline level, shading; the paragraph mark's `rPr` |
| `w:r`, `w:rPr` | runs: character style, fonts (`ascii`, else `hAnsi`, else `cs`), size (half points), bold, italic, underline, strike, color, highlight or shading, vertical alignment, caps, language |
| `w:t`, `w:delText` | text, whitespace kept (the XML reader hands whitespace-only nodes through) |
| `w:tab`, `w:br`, `w:cr`, `w:noBreakHyphen`, `w:sym` | tab, line or page break, line break, a non-breaking hyphen, a symbol character |
| `w:hyperlink` | link by relationship id or `#anchor` |
| `w:fldSimple`, `w:fldChar` + `w:instrText` | `HYPERLINK` (with `\l` as a bookmark), `PAGE`, `NUMPAGES`; other fields keep their result text |
| `w:ins`, `w:del` | revisions with author and date; deleted runs stay in the model as deleted |
| `w:footnoteReference`, `w:endnoteReference` | note reference; the space Word writes after a note mark inside the note is dropped |
| `w:drawing` | inline picture, anchored picture (floating placement), a page-anchored picture as a floating object, a text box (`wps:txbx` or a group's) as a floating text box with its fill |
| `mc:AlternateContent` | the `mc:Choice` branch; `mc:Fallback` is skipped |
| `w:tbl` | grid columns, header rows (`tblHeader`, leading rows), row heights, cells with `gridSpan` (covered cells `Merge::Left`) and `vMerge` (covered cells `Merge::Above`, origin row span), cell shading, nested tables |
| `w:sectPr` | page size and margins, header and footer distances, columns, continuous start, references; a paragraph's `sectPr` closes its section |
| `m:oMath` | the formula's text, one run |
| `w:sdt`, `w:smartTag`, `w:bookmarkStart` | transparent (`w:sdtPr` skipped) |
| `w:pict`, `w:object`, `w:commentReference` | dropped |

Units: twentieths of a point to points, half points to points, EMU to
points, hex colors.

## Known deviations

Into Word from Markdown (`markdown -> docx`, declared conditional):

- Raw HTML is dropped. Images given as `data:` URIs are embedded; any
  other image becomes a link named by its alt text.
- Loose lists come out tight; a list item's later paragraphs sit under
  the item as indented paragraphs and read back outside the list.
- Blocks other than paragraphs and lists inside a list item, and blocks
  other than paragraphs inside a block quote, lose their container.
- Link titles, code block languages, empty headings, empty items, empty
  links, and the difference between a soft break and a space are not
  carried. Two block quotes that touch merge. Task list markers become
  the box characters.
- Emphasis nested in itself flattens; Word has one italic.

Out of Word:

- Comments are not read (Apple exports mark their ranges with an empty
  line break at the paragraph end, which the projection drops).
- A paragraph style's own `numPr` (Word's "List Bullet" styles) does not
  make its paragraphs list items; only direct `numPr` does.
- Endnotes are footnotes in the model.
- Office Math loses its structure; the text is kept.
- Pictures Word cannot show inline (VML `w:pict`, embedded objects) are
  dropped; the fallback branch of `mc:AlternateContent` is never read.
- Character spacing, kerning, shadow, outline, emboss, and text effects
  have no model fields and are dropped.
