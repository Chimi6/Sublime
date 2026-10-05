#!/bin/bash
# p2w.sh <in.pages> <name>: Pages->Word check. Truth = Pages' PDF; ours = sublime docx via LibreOffice;
# apple = Pages' own Word export via LibreOffice. Side-by-side pages into p2w/.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
in="$1"; n="$2"; D="$SC/p2w/$n"; rm -rf "$D"; mkdir -p "$D"
osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1; sleep 1; killall Pages 2>/dev/null; sleep 1
open -a Pages "$in" --args -ApplePersistenceIgnoreState YES
for i in $(seq 1 25); do sleep 1; c=$(osascript -e 'tell application "Pages" to count documents' 2>/dev/null); [ "${c:-0}" -gt 0 ] && break; done
for j in 1 2 3 4 5 6; do osascript -e "tell application \"Pages\" to export front document to POSIX file \"$D/truth.pdf\" as PDF" >/dev/null 2>&1; [ -s "$D/truth.pdf" ] && break; sleep 2; done
for j in 1 2 3 4 5 6; do osascript -e "tell application \"Pages\" to export front document to POSIX file \"$D/apple.docx\" as Microsoft Word" >/dev/null 2>&1; [ -s "$D/apple.docx" ] && break; sleep 2; done
osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1
"$ROOT/target/release/sublime" convert "$in" "$D/ours.docx" -q 2>"$D/err.txt" || { echo "$n: CONVERT-FAILED $(head -c 200 $D/err.txt)"; exit 1; }
for k in ours apple; do [ -s "$D/$k.docx" ] && soffice --headless --convert-to pdf --outdir "$D" "$D/$k.docx" >/dev/null 2>&1; done
for k in truth ours apple; do [ -s "$D/$k.pdf" ] && "$SC/render" "$D/$k.pdf" "$D/$k" >/dev/null 2>&1; done
pc(){ ls "$D/$1"-p*.png 2>/dev/null | wc -l | tr -d ' '; }
t=$(pc truth); o=$(pc ours); a=$(pc apple); m=$(( t>o ? t : o )); m=$(( m>a ? m : a )); [ $m -gt 4 ] && m=4
for i in $(seq 1 $m); do
  for k in truth ours apple; do f="$D/$k-p$i.png"; [ -f "$f" ] || magick -size 918x1188 xc:'#fdd' "$f"; done
  magick \( "$D/truth-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) \( "$D/ours-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) \( "$D/apple-p$i.png" -resize 918x1188! -bordercolor '#888' -border 2 \) +append \
    -gravity north -background white -splice 0x40 -pointsize 26 -annotate +0+6 "$n p$i  —  LEFT: Pages (truth)   MIDDLE: Sublime->Word   RIGHT: Apple's Word export" "$D/cmp-p$i.png"
done
ro=$( [ -s "$D/ours.pdf" ] && python3 "$HERE/recall2.py" "$D/truth.pdf" "$D/ours.pdf" | cut -c1-120 ); ra=$( [ -s "$D/apple.pdf" ] && python3 "$HERE/recall2.py" "$D/truth.pdf" "$D/apple.pdf" | cut -c1-14 )
echo "$n: pages truth=$t ours=$o apple=$a  ours:$ro  apple:$ra"
