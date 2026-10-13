#!/bin/bash
# o2w.sh: ODT -> Word. The reference is LibreOffice's rendering of its own
# ODT of each Word file (lo/); our Word file from that ODT, LibreOffice's,
# and pandoc's are rendered by LibreOffice. Lines into o2w.txt, then a
# summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${ODT_CHECK_DIR:-${TMPDIR:-/tmp}/odt-check}"; B="$ROOT/target/release/sublime"
RTF="$ROOT/scripts/rtf-check"
out="$SC/o2w"; rm -rf "$out"; mkdir -p "$out/ref" "$out/ours" "$out/lo" "$out/pandoc" "$out/pdf"
for f in "$SC/lo"/*.odt; do
  n=$(basename "$f" .odt)
  cp "$f" "$out/ref/$n.odt"
  "$B" -q convert "$f" "$out/ours/$n.docx" 2>/dev/null || echo "FAIL $n" >&2
  pandoc "$f" -o "$out/pandoc/$n.docx" 2>/dev/null
done
(cd "$out/ref" && soffice --headless --convert-to docx --outdir "$out/lo" *.odt >/dev/null 2>&1)
for kind in ref ours lo pandoc; do
  mkdir -p "$out/pdf/$kind"
  (cd "$out/$kind" && soffice --headless --convert-to pdf --outdir "$out/pdf/$kind" * >/dev/null 2>&1)
done
for f in "$SC/lo"/*.odt; do
  n=$(basename "$f" .odt)
  for kind in ref ours lo pandoc; do
    p="$out/pdf/$kind/$n.pdf"; [ -f "$p" ] && "$B" -q convert "$p" "$out/pdf/$kind/$n.txt" 2>/dev/null
  done
  python3 "$RTF/score.py" "$out/pdf" "$n" ref ours lo pandoc
done > "$SC/o2w.txt"
python3 "$RTF/summary.py" "$SC/o2w.txt"
