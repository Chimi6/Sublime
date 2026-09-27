# PDF

The Portable Document Format. Sublime writes PDFs of images today: a
page of each image, at the image's own size. Reading PDF (the images on
a page, then text) is the next step (`temp/pdf-0.23-plan.md`).

## Status

- Every image format to `pdf`: shipped, lossless. A JPEG goes in as it
  is (`/DCTDecode`, its bytes unchanged, so nothing is decoded or
  re-encoded); every other image is deflated with PNG predictors, its
  alpha a soft mask.
- `sublime convert a.png b.jpg c.tif scan.pdf`: three or more
  positionals, the last a `.pdf`, no `--to`: the images merge into one
  PDF in the order given, a page each. Anything other than an image is
  refused before a byte is written, and the PDF appears only when every
  page is in (a `.part` file renamed into place).
- Reading PDF: not yet.

Oracles (`tests/pdf_write.rs`, `tests/cli.rs`): every cross-reference
offset lands on its object; each page's image inflates and un-predicts
to the source's pixels (gray, gray and alpha, RGB, RGBA, the alpha as
the soft mask); a JPEG is embedded byte for byte; a merge holds a page
per input at each image's size. poppler's `pdfimages` extracts the same
pixels, and `pdfinfo` reads the page sizes.

## What the writer does

| Part | Written |
|---|---|
| Header | `%PDF-1.7` and a binary comment line |
| Pages | one per image: `/MediaBox [0 0 w h]`, a point per pixel (72 per inch; the hub carries no resolution), content `q w 0 0 h 0 0 cm /Im0 Do Q` |
| Pixels | `/FlateDecode` with `/DecodeParms << /Predictor 15 >>`: the rows filtered as our PNG writer filters them, deflated in 256 KiB parts; `/DeviceGray` or `/DeviceRGB`, 8 bits |
| Alpha | an `/SMask` gray image, compressed in memory while the color streams (it is its own object, after the color) |
| JPEG | the file itself; `/DeviceGray`, `/DeviceRGB`, or `/DeviceCMYK` from its frame header, with `/Decode [1 0 1 0 1 0 1 0]` for Adobe's inverted CMYK |
| Structure | objects in file order with their offsets recorded; a stream's `/Length` an indirect object written after it, so no stream is held; the page tree (object 2), the catalog, a cross-reference table, the trailer |

## Known deviations

- Page size is a point per pixel; an image's DPI is not carried yet.
- No document information dictionary, metadata stream, or outline.
