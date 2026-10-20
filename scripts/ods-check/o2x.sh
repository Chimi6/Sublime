#!/bin/bash
# o2x.sh DIR: ODS -> Excel by sublime for every .ods in DIR/src, then
# LibreOffice's CSV export of every sheet of each workbook we wrote
# (DIR/xback/), scored against its export of the ODS itself (DIR/ref/):
# values and number formats must show the same.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
DIR="$1"; rm -rf "$DIR/x" "$DIR/xback"; mkdir -p "$DIR/x" "$DIR/xback"
for f in "$DIR"/src/*.ods; do
  n=$(basename "$f" .ods)
  "$ROOT/target/release/sublime" -q convert "$f" "$DIR/x/$n.xlsx" 2>/dev/null || echo "failed: $n"
done
soffice --headless --convert-to 'csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,-1' \
  --outdir "$DIR/xback" "$DIR"/x/*.xlsx > "$DIR/xback.log" 2>&1
mkdir -p "$DIR/xback/src" && ln -sf "$DIR"/src/*.ods "$DIR/xback/src/" 2>/dev/null
python3 "$HERE/score.py" "$DIR/ref" "$DIR/xback" "$DIR/src" 30 flat
