# TIFF

The Tagged Image File Format: a header naming the byte order, then
image file directories (IFDs) of tagged values pointing at the pixel
data, in strips of rows or in tiles. Sublime reads the first page into
the image hub and writes TIFF from it (`src/io/tiff.rs`).

## Status

- `tiff` (`.tif`, `.tiff`) to and from every image format: shipped.
  Reading takes the first page (reporting the others as dropped),
  keeps 16-bit gray and RGB at 16 bits into PNG, TIFF, and Netpbm (and
  scales them to 8 elsewhere), converts CMYK to RGB without a profile,
  and drops metadata (resolution, EXIF, ICC, XMP). Writing is lossless.

Oracles (`tests/tiff_suite.rs`, fixtures in `tests/fixtures/tiff`
written by ImageMagick, with a premultiplied-alpha file built here):
all 139 files read to the pixels they mean; every image survives our
writer and reader; Pillow and ImageMagick read our files to the same
pixels; all 6 corrupt and unsupported files are refused.

## What the reader does

| Part | Read |
|---|---|
| Header | little- and big-endian (`II`, `MM`); the first IFD; later pages counted |
| Layout | strips and tiles; chunky and planar (8 and 16-bit) |
| Compression | none, LZW (libtiff's form, codes growing one early), deflate (both tag values), PackBits |
| Predictor | horizontal differencing at 8 and 16 bits |
| Samples | 1, 2, 4, 8, and 16 bits unsigned; 16-bit gray and RGB go on whole to a 16-bit writer, else by their high byte, as for PNG; associated alpha is divided out at 16 bits too |
| Color | min-is-white and min-is-black gray, palette (the color map's high bytes), RGB, CMYK (as Pillow converts it), with unassociated or associated (premultiplied, divided out) alpha |

The file is held (its strips may lie anywhere in it); a band of rows,
a strip or a row of tiles, is decoded at a time and handed to the sink.

## What the writer does

Little-endian, one page: 8-bit gray, gray and alpha, RGB, or RGBA in
strips of about 256 KiB of pixels, deflate (zlib) with the horizontal
predictor, 72 dpi, the directory ahead of the data. The compressed
strips are held until the last row, since their offsets go in the
directory.

## Known deviations

- Not read: JPEG and old-JPEG compression, CCITT fax, JPEG 2000,
  Zstandard, YCbCr, CIE Lab, floating-point and signed samples, the
  floating-point predictor, BigTIFF, tiled or planar data under 8 bits,
  old-style LZW.
- Only the first page is read; multi-page output is not written.
