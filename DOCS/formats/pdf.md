# PDF

The Portable Document Format. Sublime writes PDFs of images (a page of
each image, at the image's own size) and reads a page's image back out
(`src/io/pdf`). Text (0.24) is next; rendering vector pages is not
planned.

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
| Pages | one per image: `/MediaBox [0 0 w h]`, a point per pixel (72 per inch; the hub carries no resolution), content `q w 0 0 h 0 0 cm /Im0 Do Q` |
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

## Known deviations

- Page size is a point per pixel; an image's DPI is not carried yet.
- Not read: encrypted files (even with an empty password), JPEG 2000,
  CCITT fax and JBIG2 images (common for black-and-white scans),
  Separation, DeviceN, and Lab color, and a PDF's text or vector
  drawing. An image is taken as stored: the page's transformation
  (scaling, rotation, placement) is not applied.
