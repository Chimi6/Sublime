#!/bin/bash
# compare.sh <input.docx> <name>: LibreOffice reference vs Sublime->Pages, side by side per page.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
in="$1"; name="$2"; mkdir -p "$SC/cmp" "$SC/ref"
rm -f "$SC/ref/$name"* "$SC/cmp/$name"*
tmp="$SC/ref/lo-$$"; mkdir -p "$tmp"; cp "$in" "$tmp/$name.docx"
soffice --headless --convert-to pdf --outdir "$SC/ref" "$tmp/$name.docx" >/dev/null 2>&1; rm -rf "$tmp"
"$SC/render" "$SC/ref/$name.pdf" "$SC/ref/$name" >/dev/null 2>&1
mkdir -p "$SC/logs"; killall Pages 2>/dev/null; sleep 1
/usr/bin/log stream --style compact --predicate 'process == "Pages" AND (eventMessage CONTAINS[c] "needs repair" OR eventMessage CONTAINS "modified during read" OR (eventMessage CONTAINS "Assertion failure" AND NOT eventMessage CONTAINS "backtrace"))' > "$SC/logs/$name.log" 2>&1 & LP=$!; sleep 1
"$HERE/e2e.sh" "$in" "$name" >/dev/null 2>&1; status=$?
sleep 1; kill $LP 2>/dev/null; wait $LP 2>/dev/null
issues=$(grep -v "^Filtering" "$SC/logs/$name.log" | grep -E "needs repair|modified during read|Assertion failure #" | grep -vc "TPPaginatedPageController\|TPInteractiveCanvasController")
rp=$(ls "$SC/ref/$name"-p*.png 2>/dev/null | wc -l | tr -d ' '); op=$(ls "$SC/out/$name"-p*.png 2>/dev/null | wc -l | tr -d ' ')
n=$(( rp > op ? rp : op ))
for i in $(seq 1 $n); do
  a="$SC/ref/$name-p$i.png"; b="$SC/out/$name-p$i.png"
  [ -f "$a" ] || magick -size 918x1188 xc:'#fdd' "$a"; [ -f "$b" ] || magick -size 918x1188 xc:'#fdd' "$b"
  magick \( "$a" -resize 918x1188! -bordercolor '#888' -border 2 \) \( "$b" -resize 918x1188! -bordercolor '#888' -border 2 \) +append \
    -gravity north -background white -splice 0x40 -pointsize 26 -annotate +0+6 "$name  page $i   —   LEFT: reference (LibreOffice)    RIGHT: Sublime -> Pages" "$SC/cmp/$name-p$i.png"
done
recall=$( [ -f "$SC/out/$name.pdf" ] && python3 "$HERE/recall.py" "$in" "$SC/out/$name.pdf" | cut -c1-12 )
echo "$name: pages ref=$rp ours=$op  status=$status  issues=$issues  $recall"
