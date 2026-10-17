#!/bin/bash
# x2o.sh DIR: Excel -> ODS. LibreOffice saves every .ods in DIR/src as a
# workbook (DIR/xl/, real Excel number formats of every kind) and exports
# each workbook's sheets to CSV (DIR/xlref/); sublime converts each workbook
# to ODS (DIR/xo/), whose export by LibreOffice (DIR/xoback/) must show the
# same.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
DIR="$1"; rm -rf "$DIR/xl" "$DIR/xlref" "$DIR/xo" "$DIR/xoback"
mkdir -p "$DIR/xl" "$DIR/xlref" "$DIR/xo" "$DIR/xoback"
FILTER='csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,-1'
soffice --headless --convert-to xlsx --outdir "$DIR/xl" "$DIR"/src/*.ods > /dev/null 2>&1
soffice --headless --convert-to "$FILTER" --outdir "$DIR/xlref" "$DIR"/xl/*.xlsx > /dev/null 2>&1
for f in "$DIR"/xl/*.xlsx; do
  n=$(basename "$f" .xlsx)
  "$ROOT/target/release/sublime" -q convert "$f" "$DIR/xo/$n.ods" 2>/dev/null || echo "failed: $n"
done
soffice --headless --convert-to "$FILTER" --outdir "$DIR/xoback" "$DIR"/xo/*.ods > /dev/null 2>&1
# Formula cells are known from the ODS each workbook came from.
mkdir -p "$DIR/xlsrc"; for f in "$DIR"/xl/*.xlsx; do n=$(basename "$f" .xlsx); ln -sf "$DIR/src/$n.ods" "$DIR/xlsrc/$n.ods"; done
python3 "$HERE/score.py" "$DIR/xlref" "$DIR/xoback" "$DIR/xlsrc" 30 flat
