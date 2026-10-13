# odt-check

Checks ODT <-> Word conversions against LibreOffice (the reference
OpenDocument implementation) and pandoc, on macOS. None of it runs in CI:
it needs LibreOffice (`soffice`), pandoc, and a release build
(`cargo build --release`). Scoring is shared with `scripts/rtf-check`.

Everything is written under `$ODT_CHECK_DIR` (default `$TMPDIR/odt-check`).
The corpus is the repository's Word fixtures plus every `.docx` in
`$ODT_CHECK_CORPUS` (default `~/Downloads`).

- `corpus.sh`: copies the Word files to `src/` and saves each as ODT by
  LibreOffice (`lo/`, the native ODT) and by pandoc (`pandoc/`).
- `w2o.sh`: Word -> ODT. The reference is LibreOffice's rendering of the
  Word file; our ODT, LibreOffice's own, and pandoc's are rendered by
  LibreOffice. Page counts and word recall per file, then a summary.
- `o2w.sh`: ODT -> Word. The reference is LibreOffice's rendering of its
  own ODT (`lo/`); our Word file from it, LibreOffice's, and pandoc's are
  rendered the same way.

Recall is word overlap with the reference's text, read back from each PDF
with `sublime` (see `scripts/rtf-check/README.md`).
