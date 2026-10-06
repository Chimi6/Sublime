#!/bin/bash
# w2r.sh: Word -> RTF. The reference is LibreOffice's rendering of each Word
# file; ours, LibreOffice's own (lo/), and Apple's (cocoa/) RTF are rendered
# by LibreOffice. Lines into w2r.txt, then a summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${RTF_CHECK_DIR:-${TMPDIR:-/tmp}/rtf-check}"; B="$ROOT/target/release/sublime"
out="$SC/w2r"; rm -rf "$out"; mkdir -p "$out/ref" "$out/ours" "$out/lo" "$out/apple" "$out/pdf"
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  cp "$f" "$out/ref/$n.docx"
  "$B" -q convert "$f" "$out/ours/$n.rtf" 2>/dev/null || echo "FAIL $n" >&2
  cp "$SC/lo/$n.rtf" "$out/lo/$n.rtf" 2>/dev/null
  cp "$SC/cocoa/$n.rtf" "$out/apple/$n.rtf" 2>/dev/null
done
for kind in ref ours lo apple; do
  mkdir -p "$out/pdf/$kind"
  (cd "$out/$kind" && soffice --headless --convert-to pdf --outdir "$out/pdf/$kind" * >/dev/null 2>&1)
done
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  for kind in ref ours lo apple; do
    p="$out/pdf/$kind/$n.pdf"; [ -f "$p" ] && "$B" -q convert "$p" "$out/pdf/$kind/$n.txt" 2>/dev/null
  done
  python3 "$HERE/score.py" "$out/pdf" "$n" ref ours lo apple
done > "$SC/w2r.txt"
python3 "$HERE/summary.py" "$SC/w2r.txt"
