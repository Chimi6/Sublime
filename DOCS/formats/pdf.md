# PDF

The Portable Document Format. Sublime writes PDFs of images (a page of
each image, at the image's own size) and reads a page's image back out
(`src/io/pdf`), and reads its text. Markdown, HTML, and Word from a
PDF's text (0.24) are next; rendering vector pages is not planned.

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
  that draws no text (a scan) is reported, not failed. Other document
  formats go through text until their direct paths land.

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
| Layout | `src/io/pdf/text.rs`: glyphs on one baseline (within half their size) are a line; a gap over 0.15 of the size is a word break unless a space is already there; runs of spaces collapse. A line more than 1.6 sizes below the last, or of another size or weight, starts a block. Reading order is drawing order |

Oracle: `tests/pdf_text.rs` against pdftotext on every fixture in
`tests/fixtures/pdf-text` (PyMuPDF's base-14 and embedded TrueType
Identity-H files, an HTML story with headings and lists, two columns,
several pages and a text-less one; Ghostscript's Type 1 re-encoded to
Latin-1 and its rewrites of three of the others; and a hand-built file
for TJ kerning without spaces, `'`, `"`, `Tz`, rise, `cm` scaling,
ToUnicode ligatures, a form XObject, hex strings, and WinAnsi's upper
range): the same lines page by page, whitespace collapsed; one fixture
compared word by word where pdftotext breaks a bullet onto its own
line (the reason is in the test).

## Known deviations

- WebP, QOI, TGA, Netpbm, and ICO record no resolution a page could use (WebP's lives in EXIF, which is not read), so their pages are a point per pixel.
- Text is read in drawing order: a page that draws its columns or
  boxes out of reading order reads out of order (pdftotext reorders by
  blocks). Rotated and vertical text is read glyph by glyph in drawing
  order and reported. A Type0 font with neither ToUnicode nor a known
  character collection has only glyph ids and reads as nothing.
- Not read: encrypted files (even with an empty password), JPEG 2000,
  CCITT fax and JBIG2 images (common for black-and-white scans),
  Separation, DeviceN, and Lab color, and a PDF's vector drawing. An image is taken as stored: the page's transformation
  (scaling, rotation, placement) is not applied.
