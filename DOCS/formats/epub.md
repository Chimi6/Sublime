# EPUB (epub)

An EPUB is a ZIP of XHTML chapters with their pictures, style sheets, and
fonts. Its first entry is `mimetype` (`application/epub+zip`), stored, which
is how a file is recognised; `META-INF/container.xml` names the package
document (the OPF), whose manifest lists every file and whose spine orders
the chapters. Sublime reads a book into the document model
(`src/io/epub/reader.rs`, the chapters through the HTML reader) and writes
the model as an EPUB 3 book with an EPUB 2 table of contents for older
readers (`src/io/epub/writer.rs`). EPUB reaches and is reached from every
document format: Word, Pages, RTF, ODT, Markdown, HTML, text, and PDF
(both ways).

## Status

- `epub -> docx`, `pages`, `rtf`, `odt`, `markdown`, `html`, `text`, `pdf`: shipped.
- `docx`, `pages`, `rtf`, `odt`, `markdown`, `html`, `text`, `pdf` `-> epub`: shipped.

Measured with `scripts/epub-check` against pandoc:

Word files are the 89 of the RTF and ODT checks (the repository's fixtures
and a corpus of real documents); held-out Word files are 21 others
(Cocoa RTF and Markdown saved as Word by LibreOffice and pandoc). Books
are 23 public-domain EPUBs from Project Gutenberg and Standard Ebooks,
split before any work into 12 for development and 11 held out. A book is
read by a plain HTML parser neither tool shares; a Word file's text is
LibreOffice's, with its notes. "All kept" counts the files whose every
note, picture, and heading the conversion keeps.

| Path | Word recall (ours, pandoc) | All notes kept | All pictures kept | All headings kept | Fail epubcheck |
|---|---|---|---|---|---|
| Word -> EPUB, 89 files | 0.983, 0.983 | 89, 89 | 89, 89 | 88, 59 | 0, 82 |
| Word -> EPUB, 21 held out | 0.999, 0.999 | 21, 21 | 21, 21 | 21, 16 | 0, 21 |
| EPUB -> Word, 12 books | 1.000, 1.000 | 12, 10 | 12, 8 | 10, 1 | |
| EPUB -> Word, 11 held out | 0.999, 0.998 | 11, 8 | 11, 8 | 9, 0 | |

The held-out runs found two faults, fixed before these numbers: Project
Gutenberg's notes (`div.footnotes`, `sup.footnoteref`) were read as text
to leave out, and a Word link to `#irc://...` was taken for a web link.
The headings a book loses are its endnotes section's title, which goes
with the notes it titled. Pandoc's books fail epubcheck mostly for a
missing title (it writes none unless given `--metadata title`, which the
check does not pass) and for empty table-of-contents entries; given a
title, 53 of the 89 and 5 of the 21 would still fail on the rest; its Word files lose a Standard Ebooks book's endnotes
(left as text) and a cover the book names.

Oracles (`tests/epub_document.rs`): pandoc's EPUBs of the Word fixtures
read with their notes, pictures, lists, and tables; books built in the
test in Standard Ebooks' shape (a cover in the manifest, a navigation
document and an imprint out of reading order, endnotes in a file of their
own) and in EPUB 2's (an NCX, the cover named by a `meta`, a cover page
that shows it) read in order with their notes and cover once; our own EPUB
of every Word source reads back to the same Markdown, in a package whose
every file is in the manifest and well formed; and every `--effort` writes
the same book.

## Reading

| Part | Read as |
|---|---|
| `container.xml`, the OPF | The spine in order; items marked `linear="no"` and the navigation document (`properties="nav"`) are left out, as a reading system leaves them out of the flow. |
| Chapters | XHTML through the HTML reader (headings, paragraphs, emphasis, lists, tables with merged cells, links, pictures, code, quotes). Each chapter after the first starts a new page. |
| Notes | References marked `epub:type="noteref"` or `role="doc-noteref"` (pandoc's, Standard Ebooks', cmark-gfm's), notes standing alone (`aside epub:type="footnote"`) or in a section of them (`epub:type="endnotes"`, `role="doc-endnotes"`), in the chapter or in a notes file of their own. Links back and the notes section's heading are markup, not text. |
| Pictures | From the package, relative to their chapter, escapes decoded; a picture in an SVG frame (Gutenberg's cover page) is the picture. The cover (`properties="cover-image"`, or EPUB 2's `meta name="cover"`) comes first unless a page shows it already. |
| Links | Links out of the book stay links; links between its own pages (another chapter, a place in this one) keep their text, since no other format can follow them. |

Style sheets are not applied: a book's look is its reader's, and the
structure carries the meaning. Metadata (title, author) has no place in
the document model.

## Writing

| Model | EPUB |
|---|---|
| Chapters | The body splits at page breaks, new-page sections, and its top headings (the highest level that comes more than once, so a title alone above the chapters stays with them). |
| Styles | Each named paragraph style used is a CSS class with its alignment, indents, spacing, line height, font, size, and colour; sizes and spacing in `em` of the body text, so a reader's size setting still scales them. A style that states no spacing keeps the reader's. |
| Formatting | Bold, italic, underline, strike, superscript, and subscript as tags wherever they show (style-given ones too, so a reader without the style sheet keeps them; a heading's own weight is its heading's); size, colour, font, highlight, caps, and letter spacing as inline style. |
| Headings | `h1` to `h6` with ids, in the navigation document and the NCX three levels deep. |
| Lists | Nested `ul` and `ol` with their numbering kind and start. |
| Tables | Header rows, merged cells (`colspan`, `rowspan`), shading, vertical alignment, column widths as shares, grid lines when the source draws any. |
| Notes | EPUB 3 footnotes (`epub:type="noteref"` and `aside epub:type="footnote"`) at the end of their chapter, numbered through the book: reading systems show them as pop-ups. |
| Pictures | In the package once, at their share of the text column's width. PNG, JPEG, GIF, SVG, and WebP (EPUB's core media types) are carried; others (Windows metafiles) leave their description. |
| Equations | MathML, the chapter marked `mathml`. |
| Text boxes | A block after the paragraph they are anchored in. |
| Embedded fonts | In `fonts/`, with an `@font-face` rule per face. |
| Package | `mimetype` stored first, `content.opf` (a stable identifier from the content, the title from the title paragraph or first heading, the language from the body style, `dcterms:modified`), `nav.xhtml`, `toc.ncx`, `style.css`. `--effort` applies to the compression. |

Every book the corpus makes passes epubcheck with no errors or warnings.

## Not carried

Pages (sizes, margins), headers and footers, page-number fields, comments,
deleted text, links to the document's own bookmarks (the model does not
carry bookmarks), charts and drawn shapes, and pictures in formats EPUB
does not carry. Reading leaves out style sheets and metadata. Fixed-layout
EPUBs read as their text; Kindle formats (MOBI, AZW3) are a later step.
