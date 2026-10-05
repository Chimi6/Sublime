# pages-check

Checks Word <-> Apple Pages conversions in Apple Pages itself, on macOS.
None of it runs in CI: it drives the Pages app, so it needs Pages,
LibreOffice (`soffice`, the stand-in for Word's rendering), ImageMagick
(`magick`), `swiftc` (the PDF renderer is compiled on first use), and a
release build (`cargo build --release`).

Everything is written under `$PAGES_CHECK_DIR` (default
`$TMPDIR/pages-check`). The user-document corpus is `$PAGES_CHECK_CORPUS`
(default `~/Downloads`, every `.docx` there); the Apache POI test documents
go in `$PAGES_CHECK_DIR/web/poi`, and Pages files for the Pages -> Word sweep
in `$PAGES_CHECK_DIR/p2wsrc` (the fixtures in `tests/fixtures/pages` are
always included).

Pages repairs a file it does not like silently, so the logs matter more than
whether a file opens: `compare.sh` streams Pages' log while the file opens
and counts `needs repair`, `modified during read`, and assertion lines as
`issues`. A clean result is `issues=0`.

## Word -> Pages

- `compare.sh <in.docx> <name>`: converts, opens the result in Pages,
  exports a PDF, renders it next to LibreOffice's rendering of the source
  (`cmp/<name>-pN.png`), and prints page counts, `issues`, and word recall.
- `fullrun.sh`: `compare.sh` over the whole corpus, into `fullrun.txt`.
- `w2p.sh <in.docx> <name>`: the three-way comparison: LibreOffice, ours, and
  Apple's own import of the same file (`w2p/<name>/cmp-pN.png`).
- `w2pall.sh`, `w2ppoi.sh`: `w2p.sh` over the fixtures, the corpus, and the
  POI set, into `w2p.txt`. Apple's import fails now and then (menu
  automation); `w2pretry.sh` reruns the rows with `apple=0`.

## Pages -> Word

- `p2w.sh <in.pages> <name>`: Pages' own PDF (the truth), ours rendered by
  LibreOffice, and Apple's Word export rendered by LibreOffice
  (`p2w/<name>/cmp-pN.png`), with page counts and recall against the truth.
- `p2wall.sh`: every Pages file, into `p2w.txt`.

## Helpers

- `pagestest.sh <file.pages> [out.pdf]`: opens a file in Pages and reports
  OPENED, CRASHED (a new crash report appeared), or NOT-OPENED, exporting a
  PDF when asked.
- `appleimport.sh`, `applesave.sh`: Pages' own import of a Word file,
  exported as PDF or saved as `.pages` (imported documents are not
  scriptable, so these drive the menus).
- `recall.py`, `recall2.py`: the share of a source's words that reach a
  rendered PDF (ligature-proof).
- `btdecode.py <log> <n>`: decodes an assertion's backtrace from a Pages log
  and symbolicates it against the app's frameworks.
- `storscan.py <inspect dump>`: lists each text storage in a `sublime inspect`
  dump with its text and attribute tables, for digging into an encoding.
- `render.swift`: renders a PDF's pages to PNG (PDFKit).

To learn how Pages encodes something, make or import a document that has it
in Pages, save it (`applesave.sh`), and read it with the dev-tools build:
`cargo build --release --features dev-tools` then
`sublime inspect file.pages --object <id> --depth 12`.
