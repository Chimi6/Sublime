#!/bin/bash
# check.sh: Numbers -> CSV against numbers-parser. Fetches numbers-parser
# (MIT) and its test documents into $NUMBERS_CHECK_DIR, dumps each
# document's cell values with it, converts each with ours into a CSV per
# table, and scores cell by cell (score.py). Needs python3 and git; not run
# in CI.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${NUMBERS_CHECK_DIR:-${TMPDIR:-/tmp}/numbers-check}"; B="$ROOT/target/release/sublime"
mkdir -p "$SC"
[ -d "$SC/numbers-parser" ] || git clone -q --depth 1 https://github.com/masaccio/numbers-parser "$SC/numbers-parser"
[ -x "$SC/venv/bin/python" ] || { python3 -m venv "$SC/venv" && "$SC/venv/bin/pip" -q install numbers-parser; }
rm -rf "$SC/ref" "$SC/ours"; mkdir -p "$SC/ref" "$SC/ours"
for f in "$SC/numbers-parser/tests/data"/*.numbers; do
  n=$(basename "$f" .numbers)
  "$SC/venv/bin/python" "$HERE/values.py" "$f" "$SC/ref/$n.json" 2>/dev/null || continue
  mkdir -p "$SC/ours/$n"
  "$B" -q convert "$f" "$SC/ours/$n/out.csv" 2>/dev/null
done
"$SC/venv/bin/python" "$HERE/score.py" "$SC" "$@"
