# epub-check

Checks EPUB <-> Word conversions against pandoc, on macOS or Linux. None of
it runs in CI: it needs pandoc, LibreOffice (`soffice`, for Word files'
text), Java with epubcheck's jar (`EPUBCHECK_JAR`), and a release build
(`cargo build --release`). Everything is written under `$EPUB_CHECK_DIR`
(default `$TMPDIR/epub-check`).

- `w2e.sh`: Word -> EPUB. Every Word file (the repository's fixtures and
  `~/Downloads`, or `$EPUB_CHECK_WORD`) is made into a book by us and by
  pandoc. Both books are validated by epubcheck and read by `epubtext.py`,
  a plain HTML parser neither tool shares; their words are scored against
  LibreOffice's text of the Word file, and their notes, pictures, tables,
  and headings counted against the Word file's own XML.
- `e2w.sh`: EPUB -> Word. Every book in `$EPUB_CHECK_BOOKS` is made into
  a Word file by us and by pandoc; the reference is the book's own text
  and counts (`epubtext.py`), each Word file's are LibreOffice's text and
  its XML.

`score.py` writes one line per file (recall and precision of words, and
`got/want` counts); `summary.py` gives each tool's mean recall and
precision, how many files keep every note, picture, table, and heading,
and how many books fail epubcheck.

Books for `e2w.sh` are public-domain EPUBs from Project Gutenberg
(`https://www.gutenberg.org/ebooks/<id>.epub3.images`) and Standard Ebooks,
split before any tuning into a development set and a held-out set; the
held-out numbers are the ones reported (`DOCS/formats/epub.md`).
