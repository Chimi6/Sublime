#!/bin/bash
# e2w.sh: EPUB -> Word, ours and pandoc's, against the book. Every .epub in
# $EPUB_CHECK_BOOKS (required) is converted by both; the book's own text
# (epubtext.py) is the reference, each Word file's text is LibreOffice's.
# Lines into e2w.txt, then a summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${EPUB_CHECK_DIR:-${TMPDIR:-/tmp}/epub-check}"; B="$ROOT/target/release/sublime"
BOOKS="${EPUB_CHECK_BOOKS:?set EPUB_CHECK_BOOKS to a folder of .epub books}"
out="$SC/e2w"; rm -rf "$out"; mkdir -p "$out/ours" "$out/pandoc"
for f in "$BOOKS"/*.epub; do
  n=$(basename "$f" .epub)
  "$B" -q convert "$f" "$out/ours/$n.docx" 2>/dev/null || echo "FAIL ours $n" >&2
  pandoc -f epub -t docx "$f" -o "$out/pandoc/$n.docx" 2>/dev/null || echo "FAIL pandoc $n" >&2
done
for f in "$BOOKS"/*.epub; do
  n=$(basename "$f" .epub)
  python3 "$HERE/score.py" "$n" "$f" "$out/ours/$n.docx" "$out/pandoc/$n.docx"
done > "$SC/e2w.txt"
python3 "$HERE/summary.py" "$SC/e2w.txt"
