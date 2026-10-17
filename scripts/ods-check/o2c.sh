#!/bin/bash
# o2c.sh DIR: ODS -> CSV by sublime for every .ods in DIR/src, into
# DIR/ours/<name>.csv (one sheet) or DIR/ours/<name>/<sheet>.csv, timed.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
DIR="$1"; rm -rf "$DIR/ours"; mkdir -p "$DIR/ours"
for f in "$DIR"/src/*.ods; do
  n=$(basename "$f" .ods)
  "$ROOT/target/release/sublime" -q convert "$f" "$DIR/ours/$n.csv" 2> "$DIR/ours/$n.err" || echo "failed: $n"
  [ -s "$DIR/ours/$n.err" ] || rm -f "$DIR/ours/$n.err"
done
python3 "$HERE/score.py" "$DIR/ref" "$DIR/ours" "$DIR/src"
