# rtf-check

Checks RTF <-> Word conversions against LibreOffice, Apple's text system, and
the original documents, on macOS. None of it runs in CI: it needs LibreOffice
(`soffice`), `textutil` and `swiftc` (both ship with macOS), and a release
build (`cargo build --release`).

Everything is written under `$RTF_CHECK_DIR` (default `$TMPDIR/rtf-check`).
The corpus is the repository's Word fixtures plus every `.docx` in
`$RTF_CHECK_CORPUS` (default `~/Downloads`).

- `corpus.sh`: copies the Word files to `src/` and saves each as RTF twice:
  by LibreOffice (`lo/`, close to Word's own RTF) and by macOS's text system
  through `textutil` (`cocoa/`, as TextEdit writes it).
- `r2w.sh lo|cocoa`: RTF -> Word. The reference is the RTF as its own kind
  of reader lays it out: LibreOffice for its RTF, the macOS text system for
  Cocoa RTF (`cocoa-render.swift`, compiled on first use). Our Word output
  and Apple's (`textutil -convert docx`) are rendered by LibreOffice, the
  stand-in for Word. Page counts and word recall per file, then a summary.
- `w2r.sh`: Word -> RTF. The reference is LibreOffice's rendering of the
  Word file; ours, LibreOffice's own RTF, and Apple's are rendered the same.
- `cocoa-render.swift`: lays out an RTF with the macOS text system on the
  file's own paper and margins and writes the pages as a PDF.

Recall is word overlap with the reference's text, read back from each PDF
with `sublime`; text drawn twice (shadows, embossing) or letter by letter
(wide letter spacing) reads differently on each side, so a dip of a few
points on such files is not lost text. Diff the `pdf/<kind>/<name>.txt`
files to see what differs.
