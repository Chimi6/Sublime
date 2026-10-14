#!/bin/bash
# w2e.sh: Word -> EPUB, ours and pandoc's, against the Word file. Every
# .docx in $EPUB_CHECK_WORD (default: the repository's Word fixtures and
# ~/Downloads) is converted by both; each EPUB is read by epubtext.py and
# checked by epubcheck. Lines into w2e.txt and w2e-check.txt, then a summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${EPUB_CHECK_DIR:-${TMPDIR:-/tmp}/epub-check}"; B="$ROOT/target/release/sublime"
JAR="${EPUBCHECK_JAR:?set EPUBCHECK_JAR to epubcheck.jar}"; JAVA="${JAVA:-java}"
out="$SC/w2e"; rm -rf "$out"; mkdir -p "$out/src" "$out/ours" "$out/pandoc"
i=0
{ if [ -n "${EPUB_CHECK_WORD:-}" ]; then find "$EPUB_CHECK_WORD" -name "*.docx"; else
    find "$ROOT/tests/fixtures" -name "*.docx"; find "$HOME/Downloads" -maxdepth 1 -name "*.docx" ! -name '~$*'; fi; } |
while IFS= read -r f; do
  i=$((i+1)); n="$(printf %03d $i)-$(basename "$f" .docx | tr -c 'A-Za-z0-9_\n-' '_' | cut -c1-40)"
  cp "$f" "$out/src/$n.docx"
  "$B" -q convert "$f" "$out/ours/$n.epub" 2>/dev/null || echo "FAIL ours $n" >&2
  pandoc -f docx -t epub3 "$f" -o "$out/pandoc/$n.epub" 2>/dev/null || echo "FAIL pandoc $n" >&2
done
for tool in ours pandoc; do
  for f in "$out/$tool"/*.epub; do
    "$JAVA" -jar "$JAR" -q "$f" 2>&1 | grep -E "^(ERROR|FATAL|WARNING)" | sed "s|^|$tool $(basename "$f") |"
  done
done > "$SC/w2e-check.txt"
for f in "$out/src"/*.docx; do
  n=$(basename "$f" .docx)
  python3 "$HERE/score.py" "$n" "$f" "$out/ours/$n.epub" "$out/pandoc/$n.epub"
done > "$SC/w2e.txt"
python3 "$HERE/summary.py" "$SC/w2e.txt" "$SC/w2e-check.txt"
