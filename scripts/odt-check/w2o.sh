#!/bin/bash
# w2o.sh: Word -> ODT. The reference is LibreOffice's rendering of each Word
# file; ours, LibreOffice's own (lo/), and pandoc's ODT are rendered by
# LibreOffice. Lines into w2o.txt, then a summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${ODT_CHECK_DIR:-${TMPDIR:-/tmp}/odt-check}"; B="$ROOT/target/release/sublime"
RTF="$ROOT/scripts/rtf-check"
out="$SC/w2o"; rm -rf "$out"; mkdir -p "$out/ref" "$out/ours" "$out/lo" "$out/pandoc" "$out/pdf"
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  cp "$f" "$out/ref/$n.docx"
  "$B" -q convert "$f" "$out/ours/$n.odt" 2>/dev/null || echo "FAIL $n" >&2
  cp "$SC/lo/$n.odt" "$out/lo/$n.odt" 2>/dev/null
  cp "$SC/pandoc/$n.odt" "$out/pandoc/$n.odt" 2>/dev/null
done
for kind in ref ours lo pandoc; do
  mkdir -p "$out/pdf/$kind"
  (cd "$out/$kind" && soffice --headless --convert-to pdf --outdir "$out/pdf/$kind" * >/dev/null 2>&1)
done
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  for kind in ref ours lo pandoc; do
    p="$out/pdf/$kind/$n.pdf"; [ -f "$p" ] && "$B" -q convert "$p" "$out/pdf/$kind/$n.txt" 2>/dev/null
  done
  python3 "$RTF/score.py" "$out/pdf" "$n" ref ours lo pandoc
done > "$SC/w2o.txt"
python3 "$RTF/summary.py" "$SC/w2o.txt"
