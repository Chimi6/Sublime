# QOI

The Quite OK Image format: a 14-byte header (magic `qoif`, width,
height, channels, colorspace) and a stream of chunks, each a run of
the previous pixel, an index into the last 64 colors seen, a small
difference from the previous pixel, or a literal, ending in seven zero
bytes and a one. Sublime reads it into the image hub and writes it
from the hub (`src/io/qoi.rs`), streaming rows both ways.

## Status

- `qoi` to and from `png`, `bmp`, `jpeg`, `webp`: shipped. QOI holds
  8-bit RGB or RGBA and no metadata, so its own reading and writing
  lose nothing; a pair's fidelity is the other side's.

Oracles (`tests/qoi_suite.rs`, fixtures in `tests/fixtures/qoi` written
by Pillow with the pixels it decodes beside each as `.pix`): all 60
images (RGB and RGBA, 1 by 1 to 257 by 130, runs past 62, index hits,
small differences, noise) decode to Pillow's pixels; our writer
produces Pillow's bytes from them, save the colorspace byte; all 6
corrupt files are refused.

## What the reader does

Chunks are decoded as the input arrives, a 64 KiB piece at a time; a
row goes to the sink as it completes. The colorspace byte is
informative and ignored. Images over 2^31 pixels, zero dimensions, and
channel counts other than 3 and 4 are refused; data that ends before
the last pixel is an error. A missing end marker is not.

## What the writer does

Exactly `qoi.h`'s choices, pixel by pixel: a run while the pixel
repeats (closed at 62 and at the image's end), an index hit, a
difference, a luma difference, or a literal with or without alpha.
Gray becomes RGB and gray with alpha RGBA; RGB images write three
channels. The colorspace byte is 0, sRGB with linear alpha, which is
what the hub's pixels are.

## Known deviations

- Pillow writes colorspace 1 (all linear); ours writes 0.
