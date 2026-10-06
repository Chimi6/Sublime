#!/bin/bash
# check.sh: Numbers -> CSV and Numbers -> Excel against Numbers itself.
# Fetches numbers-parser's test documents (MIT) into $NUMBERS_CHECK_DIR,
# exports each to CSV with Numbers.app (the reference: cells as Numbers
# shows them), converts each with ours to CSV and to a workbook, renders
# the workbook to CSV with LibreOffice (cells as Excel's formats show
# them), and scores both cell by cell (score.py). macOS with Numbers and
# LibreOffice, python3 and git; not run in CI. Numbers opens each document
# while it runs. Pass -v to list differences.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${NUMBERS_CHECK_DIR:-${TMPDIR:-/tmp}/numbers-check}"; B="$ROOT/target/release/sublime"
LO_CSV='csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,-1'
mkdir -p "$SC/app" "$SC/names" "$SC/ours" "$SC/xl"
[ -d "$SC/numbers-parser" ] || git clone -q --depth 1 https://github.com/masaccio/numbers-parser "$SC/numbers-parser"
[ -x "$SC/venv/bin/python" ] || { python3 -m venv "$SC/venv" && "$SC/venv/bin/pip" -q install numbers-parser; }
for f in "$SC/numbers-parser/tests/data"/*.numbers; do
  n=$(basename "$f" .numbers)
  "$SC/venv/bin/python" "$HERE/names.py" "$f" "$SC/names/$n.json" 2>/dev/null || continue
  if [ ! -e "$SC/app/$n" ]; then
    osascript -e "with timeout of 90 seconds
      tell application \"Numbers\"
        set d to open POSIX file \"$f\"
        export d to POSIX file \"$SC/app/$n\" as CSV
        close d saving no
      end tell
    end timeout" >/dev/null 2>&1 || { rm -f "$SC/names/$n.json"; continue; }
  fi
  rm -rf "$SC/ours/$n" "$SC/xl/$n"; mkdir -p "$SC/ours/$n" "$SC/xl/$n"
  "$B" -q convert "$f" "$SC/ours/$n/out.csv" 2>/dev/null
  "$B" -q convert "$f" "$SC/xl/$n/out.xlsx" 2>/dev/null
  soffice --headless --convert-to "$LO_CSV" --outdir "$SC/xl/$n" "$SC/xl/$n/out.xlsx" >/dev/null 2>&1
done
echo "text (ours -> CSV against Numbers' CSV export):"
python3 "$HERE/score.py" "$SC" ours "$@"
echo "Excel (ours -> xlsx, shown by LibreOffice, against Numbers' CSV export):"
python3 "$HERE/score.py" "$SC" xl "$@"
