# JPEG

The photograph format: 8-bit samples in YCbCr, transformed in 8 by 8
blocks, quantized by a table scaled from a quality setting, and
Huffman-coded. Sublime reads baseline and progressive JPEG into the
image hub (`src/io/jpeg/reader.rs`) and writes baseline JPEG from it
(`src/io/jpeg/writer.rs`), since 0.20.0, so a JPEG reaches PNG and BMP
and comes from them. This is the living map of what the reader and
writer handle, tied to the tests that prove it.

## Status

- `jpeg -> png`, `jpeg -> bmp`: shipped, conditional (the pixels as
  decoded; the Exif orientation tag is reported, not applied; Exif,
  ICC profiles, XMP, and comments are dropped and reported by name).
  Streams: rows go out one MCU row at a time and no image is held
  (6 MB peak on a 46 MB image).
- `png -> jpeg`, `bmp -> jpeg`: shipped, lossy (re-encoded at
  `--quality`, 85 by default; alpha flattened onto white; metadata
  dropped). Streams: sixteen rows at a time.
- `--quality <1-100>` on `convert` and batch conversion.

Oracles (`tests/jpeg_suite.rs`, fixtures in `tests/fixtures/jpeg`
written by Pillow's libjpeg-turbo with the pixels it decodes beside
each as `.pix`): all 140 images decode bit for bit to libjpeg-turbo's
pixels, across seven sizes from 1 by 1, four kinds of content, 4:4:4,
4:2:2, and 4:2:0 chroma, qualities 10 to 100, grayscale, optimized
Huffman tables, restart intervals, and progressive scans; all 6 corrupt
files are refused; our writer's files read back close to the source,
closer as quality rises within each subsampling, and Pillow reads
them. At quality 85 on a real photograph our file is the same size
and PSNR as libjpeg-turbo's.

## What the reader does

| Part | Read |
|---|---|
| SOI, SOF0, SOF1, SOF2 | baseline, extended sequential, and progressive frames of 8-bit samples with one or three components (grayscale, YCbCr, or RGB when an Adobe segment says so); sampling factors 1 to 4 |
| DQT, DHT, DRI | quantization tables (8- and 16-bit), Huffman tables (a nine-bit lookahead table and the canonical walk past it), restart intervals with RST markers honored |
| SOS | an interleaved baseline scan streams MCU row by MCU row; scans of one component and progressive scans (DC first and refinement, AC first and refinement with end-of-band runs) collect into coefficient buffers and decode at the end |
| APP0, APP1, APP2, APP14, COM | JFIF noted; Exif parsed for the orientation tag and dropped; XMP, ICC, and comments dropped and reported; the Adobe transform flag honored |
| EOI | the end; a truncated file is refused |

Decoding matches libjpeg-turbo, which is what browsers and Pillow
produce: the accurate integer IDCT (`jidctint`, with a flat-block
shortcut), triangle ("fancy") upsampling for 2:1 horizontal, vertical,
and both, and libjpeg's fixed-point YCbCr conversion. The file is read
whole (its entropy-coded data is one bit stream) and the pixels are
never held.

## What the writer does

JFIF baseline: the standard quantization tables scaled by the IJG
formula (5000/q below 50, 200 - 2q above), so a quality number means
what it means everywhere; 4:2:0 chroma below quality 90 and 4:4:4 from
90 up (the jpeg-encoder crate's rule); the standard Huffman tables of
Annex K, which let the encoder stream; the accurate integer FDCT
(`jfdctint`) with quantization by reciprocal multiply; libjpeg's
fixed-point RGB to YCbCr; box downsampling with libjpeg's alternating
bias; edges padded by replication. Gray input writes one component.
Alpha is flattened onto white before conversion.

## Known deviations

- Not read: CMYK and YCCK (four-component Adobe files), 12-bit
  samples, arithmetic coding, lossless and hierarchical JPEG, and a
  height given late by a DNL marker.
- Exif orientation is reported and not applied: rotating needs the
  whole image, which the streaming path never holds. Applying it is a
  transform for the image path.
- The writer offers no progressive output, optimized Huffman tables,
  or restart intervals; the standard tables cost a few percent of size
  against optimized ones.
- Flat graphics with hard color edges lose more to 4:2:0 chroma than
  photographs do; the `image` crate's 4:2:2 default scores far higher
  on them at a larger file. Choosing the subsampling from the chroma's
  own detail is the lever.
