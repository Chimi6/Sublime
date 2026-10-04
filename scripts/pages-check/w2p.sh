#!/bin/bash
# w2p.sh <in.docx> <name>: Word->Pages 3-way. LEFT LibreOffice (Word stand-in), MIDDLE Sublime->Pages, RIGHT Apple's import.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
in="$1"; n="$2"; D="$SC/w2p/$n"; rm -rf "$D"; mkdir -p "$D"
cp "$in" "$D/src.docx"
soffice --headless --convert-to pdf --outdir "$D" "$D/src.docx" >/dev/null 2>&1; mv "$D/src.pdf" "$D/ref.pdf" 2>/dev/null
"$ROOT/target/release/sublime" convert "$in" "$D/ours.pages" -q 2>/dev/null
"$HERE/pagestest.sh" "$D/ours.pages" "$D/ours.pdf" >/dev/null 2>&1
"$HERE/appleimport.sh" "$D/src.docx" "$D/apple.pdf" >/dev/null 2>&1
for k in ref ours apple; do [ -s "$D/$k.pdf" ] && "$SC/render" "$D/$k.pdf" "$D/$k" >/dev/null 2>&1; done
pc(){ ls "$D/$1"-p*.png 2>/dev/null | wc -l | tr -d ' '; }
r=$(pc ref); o=$(pc ours); a=$(pc apple); m=$(( r>o ? r : o )); m=$(( m>a ? m : a )); [ $m -gt 4 ] && m=4
for i in $(seq 1 $m); do
  for k in ref ours apple; do f="$D/$k-p$i.png"; [ -f "$f" ] || magick -size 918x1188 xc:'#fdd' "$f"; done
  magick \( "$D/ref-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) \( "$D/ours-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) \( "$D/apple-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) +append \
    -gravity north -background white -splice 0x40 -pointsize 26 -annotate +0+6 "$n p$i  —  LEFT: LibreOffice (Word stand-in)   MIDDLE: Sublime->Pages   RIGHT: Apple's Pages import" "$D/cmp-p$i.png"
done
ro=$( [ -s "$D/ours.pdf" ] && python3 "$HERE/recall.py" "$in" "$D/ours.pdf" | cut -c1-12 ); ra=$( [ -s "$D/apple.pdf" ] && python3 "$HERE/recall.py" "$in" "$D/apple.pdf" | cut -c1-12 )
echo "$n: pages ref=$r ours=$o apple=$a  ours:$ro  apple:$ra"
