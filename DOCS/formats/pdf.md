# PDF

The Portable Document Format. Sublime writes PDFs of images (a page of
each image, at the image's own size) and reads a page's image back out
(`src/io/pdf`), and reads its text as plain text, Markdown, HTML, and
Word, and writes PDF from Markdown, HTML, text, Word, and Pages, in the
standard fonts with the machine's own fonts embedded for other scripts,
with the documents' pictures. Rendering vector pages is not planned.

## Status

- Every image format to `pdf`: shipped, lossless. A JPEG goes in as it
  is (`/DCTDecode`, its bytes unchanged); every other image is deflated
  with PNG predictors, its alpha a soft mask.
- `sublime convert a.png b.jpg c.tif scan.pdf`: three or more
  positionals, the last a `.pdf`, no `--to`: the images merge into one
  PDF in the order given, a page each. Anything other than an image is
  refused before a byte is written, and the PDF appears only when every
  page is in (a `.part` file renamed into place).
- `pdf` to every image format: shipped, conditional. The chosen page's
  largest image (`--page N`, 1-based, the first by default); other
  pages and images are reported as dropped. PDF to JPEG copies a plain
  JPEG image's bytes rather than re-encoding them.
- `pdf` to `text`: shipped, lossy. Every page (or `--page N`), laid out
  as pdftotext lays it out without `-layout`: a line per line of the
  page, a blank line between blocks, a form feed after each page. A page
  that draws no text (a scan) is reported, not failed. A word
  hyphenated across a line end joins, as pdftotext joins it.
- `pdf` to `markdown`, `html`, and `docx`: shipped, lossy. The text as
  structure (`src/io/pdf/markdown.rs`): the most common size is the
  body; blocks of up to three lines set at least 15% larger are
  headings, a level per larger size (three at most); a block of up to
  two lines, all bold, at most 80 characters and not ending like a
  sentence, is a heading below those; a line starting with a bullet or
  `1.`/`1)` starts a list item; a paragraph's lines join; a block that is
  only a page number is dropped.

Oracles: `tests/pdf_write.rs` (every cross-reference offset lands on its
object; each image inflates and un-predicts to the source's pixels, the
alpha to the soft mask; a JPEG is embedded byte for byte), `tests/cli.rs`
(a merge holds a page per input), and `tests/pdf_read.rs`: all 24
fixtures (Pillow's JPEG and palette PDFs; ImageMagick's Zip, LZW, RLE,
and uncompressed ones; Ghostscript's rewrites with object streams, a
cross-reference stream, and inline images; and files built here for
pages, forms, 16-bit, Decode, ASCII85 over Flate, CMYK, 4-bit palettes,
stencil masks, and a broken cross-reference table) open to their
page's image; our PDFs read back to their images; all 6 unsupported or
broken files are refused. poppler's `pdfimages` extracts the same
pixels from our PDFs, and `pdfinfo` reads their pages.

## What the writer does

| Part | Written |
|---|---|
| Header | `%PDF-1.7` and a binary comment line |
| Pages | one per image at its physical size: `/MediaBox [0 0 w h]` with w = pixels × 72 / pixels per inch, from the resolution the image records (PNG `pHYs` in metres, JPEG JFIF in inches or centimetres, TIFF `XResolution` and `ResolutionUnit`, BMP pixels per metre); a point per pixel when it records none or an implausible one (outside 1 to 10,000 per inch). Content `q w 0 0 h 0 0 cm /Im0 Do Q` |
| Pixels | `/FlateDecode` with `/DecodeParms << /Predictor 15 >>`: the rows filtered as our PNG writer filters them; `/DeviceGray` or `/DeviceRGB`, 8 bits |
| Alpha | an `/SMask` gray image, compressed in memory while the color streams |
| JPEG | the file itself; gray, RGB, or CMYK from its frame header, `/Decode [1 0 1 0 1 0 1 0]` for Adobe's inverted CMYK |
| Structure | objects in file order, offsets recorded; a stream's `/Length` an indirect object written after it, so no stream is held; the page tree, the catalog, a cross-reference table, the trailer |

## What the document writer does

| Part | Handling |
|---|---|
| Input | any reader's Markdown event stream (markdown, html, text directly; docx and pages through the document model) |
| Page | US Letter, one-inch margins; a page is written when the next line would pass the bottom margin, so one page is held |
| Fonts | the base-14 Helvetica, Helvetica-Bold, -Oblique, -BoldOblique, Courier, Courier-Bold, WinAnsi, not embedded; widths from their AFM metrics. A character outside WinAnsi is set in a font installed on the machine that has it (`src/io/font/system.rs`: broad sans serifs first, then one per script, symbols, the large CJK fallbacks, then every other font, each probed by its character map alone), embedded as a subset (Type0, Identity-H, CIDFontType2 over a TrueType program holding only the glyphs used, with `/W` widths and a ToUnicode map). `--font file.ttf` (or `ConvertOptions::font`, which the WebAssembly module takes as bytes) sets all body text in that font instead, code staying in Courier. Only fonts with TrueType outlines are embedded; a character no such font has is `?` and reported |
| Text | paragraphs 11/15 pt, broken first fit on real widths (a word wider than the line is split by characters); headings 20, 16, 13 pt bold (4 to 6 at 11), kept with the two lines after them; soft breaks are spaces, hard breaks end the line |
| Inline | bold, italic, and code change the font; links are blue and carry a URI annotation over each piece; footnote references as `[n]`; task markers as `[x]`/`[ ]` |
| Blocks | lists indented 18 pt a level with `•` or `n.`, tight ones close; block quotes indented 16 pt with a gray bar; code 9.5 pt Courier on a gray band, lines kept, long ones wrapped; rules; tables sized from their content (natural widths when they fit, else the longest word plus a share of the rest), cells wrapped and aligned, header bold, a grid, rows kept whole across pages |
| Images | each picture a block of its own at its size, scaled down to the text width and the page, on a new page when it does not fit: Word, Pages, and RTF pictures from the document, at the size it shows each (a picture shown twice at two sizes takes each); Markdown and HTML pictures by path from the input's folder (on the command line; `ConvertOptions::base`) or by `data:` URI, at their recorded resolution or 96 pixels to the inch. A JPEG is embedded unchanged (`/DCTDecode`); PNG, WebP, BMP, TIFF, QOI, and ICO are read whole first (a picture that fails leaves nothing behind) and deflated with PNG predictors, alpha as a soft mask. A picture not found, unreadable (GIF has no reader yet), in a table cell, or by web address (not fetched) keeps its alt text in brackets, and is counted |
| Document | the first heading is the `/Title` in the information dictionary, `/Producer` Sublime |

The font reader (`src/io/font`): table directories of fonts and
collections, cmap formats 4 and 12, hmtx, head, hhea, maxp, OS/2, post,
name; subsets follow composite glyphs, keep every glyph at its own id
(no renumbering), and keep cvt, fpgm, and prep for hinting; tests on
Droid Sans (`tests/fixtures/fonts`, Apache-2.0) check the map, the
advances, a subset's kept and emptied glyphs, and its checksums.

Oracle: `tests/to_pdf.rs`: a Markdown document set as PDF reads back
through our PDF reader as the identical Markdown; a 40-section document
fills over ten pages, keeps every word in order, and every line lies
inside the margins; a link is a URI annotation; a character outside
WinAnsi that no font has reads as `?`; a font given with `--font` is
subset and embedded (Greek and Cyrillic read back as written, the file
under a quarter of the font); HTML, text, Word, and Pages all reach
PDF; a Pages document's three pictures are set at their sizes, and a
Markdown document's JPEG by path (embedded unchanged) and PNG by `data:`
URI (with its alpha), a missing one keeping its alt text. Greek, Cyrillic, Hebrew, Arabic, Chinese, Japanese, Hindi, Thai,
and symbols set from this machine's fonts read back through pdftotext
as written. Every
Pages and Word fixture converts, and poppler's pdfinfo, pdftotext, and
pdftoppm read and render each without an error.

## What the reader does

| Part | Read |
|---|---|
| Structure | cross-reference tables and streams, every revision (`/Prev`, `/XRefStm`), object streams; a file whose cross references are broken is rebuilt by scanning for its objects |
| Objects | numbers, strings (literal and hex), names with `#` escapes, arrays, dictionaries, references, streams whose `/Length` is missing or wrong |
| Filters | Flate (with PNG and TIFF predictors), LZW (early change or not), ASCIIHex, ASCII85, RunLength, in chains; DCT to the JPEG reader |
| Images | the page's image XObjects, those inside forms it uses, and inline images (`BI` ... `ID` ... `EI`); the largest is read |
| Color | DeviceGray, DeviceRGB, DeviceCMYK (converted as Pillow converts), CalGray, CalRGB, ICCBased (by its component count), Indexed over any of them (palette string or stream); 1, 2, 4, 8, and 16 bits (16 by the high byte); `/Decode` arrays; stencil masks (`/ImageMask`); `/SMask` as alpha |

The file is held (its cross references are at the end). An image under
a lone Flate filter, or none, streams: rows are inflated, their
predictor undone, and their samples turned into pixels one row at a
time, the soft mask alongside, so a page reads in about the file's size
plus a few rows. JPEG images and other filter chains decode whole
first.

## What the text reader does

| Part | Handling |
|---|---|
| Content | `src/io/pdf/content.rs` runs the page's content: `q`/`Q`/`cm`, `BT`/`ET`, `Tc Tw Tz TL Tf Ts`, `Td TD Tm T*`, `Tj TJ ' "` (TJ offsets move the text position), form XObjects with their matrix and resources (8 deep), inline images skipped. Each shown code becomes a glyph placed in page space with its size there |
| Fonts | `src/io/pdf/font.rs`: code lengths from the ToUnicode code-space ranges (two bytes for Type0 without one); Unicode from ToUnicode (bfchar, bfrange with strings or arrays), else a simple font's encoding: Standard, WinAnsi, MacRoman, or the Symbol and Zapf Dingbats built-ins, with `/Differences` names through the glyph list, `uniXXXX`, `uXXXX`, a `.suffix` dropped, and ligatures (fi, fl, ff, ffi, ffl) spelled out. Widths from `/Widths` with `/FirstChar` and `/MissingWidth`, CID `/W` and `/DW`, Type3 `/FontMatrix`, and the Core 14 AFM widths for base fonts without `/Widths` (`src/io/pdf/tables.rs`, generated by `scripts/gen-pdf-tables.py`) |
| Layout | `src/io/pdf/text.rs`: glyphs on one baseline (within half their size) are a line; a gap over 0.15 of the size is a word break unless a space is already there; runs of spaces collapse. A line of another size or weight starts a block, and so does one further below the last than 1.6 sizes and than the page's most common line spacing (so a font whose glyph units are not the usual em, which Ghostscript's rewrites of CID fonts make, still groups right). Reading order is drawing order |

Oracle: `tests/pdf_text.rs` against pdftotext on every fixture in
`tests/fixtures/pdf-text` (PyMuPDF's base-14 and embedded TrueType
Identity-H files, an HTML story with headings and lists, two columns,
several pages and a text-less one; Ghostscript's Type 1 re-encoded to
Latin-1 and its rewrites of three of the others; and a hand-built file
for TJ kerning without spaces, `'`, `"`, `Tz`, rise, `cm` scaling,
ToUnicode ligatures, a form XObject, hex strings, and WinAnsi's upper
range): the same lines page by page, whitespace collapsed; one fixture
compared word by word where pdftotext breaks a bullet onto its own
line (the reason is in the test). `tests/pdf_markdown.rs`: the story
and a hand-laid report read as the Markdown they were made from
(headings at three levels, lists, a joined hyphenation, dropped page
numbers), HTML carries them, and the Word file reads back to the same
Markdown.

## Known deviations

- Writing: a picture is a block of its own (one inline with text breaks
  the line around it), its crop is not applied, and a floating one sits
  where it is anchored, not beside the text; pictures in table cells
  show their alt text; no hyphenation, page numbers,
  headers, or footers. Complex scripts are set glyph by glyph in logical
  order: Hebrew and Arabic read left to right (no bidirectional
  reordering), Arabic letters take their isolated forms, and Indic
  conjuncts are not formed (no shaping). Fonts with CFF outlines (most
  CJK `.otf`/`.ttc`) are not embedded yet; another installed TrueType
  font is used when one covers the text. A tab in running text is a
  space. With `--font`, bold and italic body text is set in the one font
  given.

- WebP, QOI, TGA, Netpbm, and ICO record no resolution a page could use (WebP's lives in EXIF, which is not read), so their pages are a point per pixel.
- Text is read in drawing order: a page that draws its columns or
  boxes out of reading order reads out of order (pdftotext reorders by
  blocks). Rotated and vertical text is read glyph by glyph in drawing
  order and reported. A Type0 font with neither ToUnicode nor a known
  character collection has only glyph ids and reads as nothing.
- Structure is guessed from type: bold or italic inside a line, tables,
  links, and footnotes are not read; a paragraph that runs across a
  page break is two paragraphs; running headers other than page numbers
  stay in the text.
- Not read: encrypted files (even with an empty password), JPEG 2000,
  CCITT fax and JBIG2 images (common for black-and-white scans),
  Separation, DeviceN, and Lab color, and a PDF's vector drawing. An image is taken as stored: the page's transformation
  (scaling, rotation, placement) is not applied.
