#!/usr/bin/env bash
# Pages -> xlsx. Sourced by bench/run.sh. Inputs are Markdown documents of
# tables written to Pages by our own writer (bench/pairs/pages-xlsx-gen.py):
# `tables`, tables of 200 rows each, and `report`, prose with a 25-row
# table every four paragraphs. `tables` is capped at 100,000 rows (500
# tables) and `report` at 25,000 (1,000 tables): the Pages writer gives
# each table its own entries and writes no ZIP64, so a package holds about
# 2,000 tables (DOCS/STATE.md). No other tool
# converts Pages to Excel (LibreOffice opens Pages as a text document, and
# Pages exports no workbook), so the reference column holds the goals every
# Pages pair shares (pages-json.sh). Excel output is compressed: the input
# plus the output's uncompressed bytes is an extra row (README).

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

pages_xlsx_inputs() {
  local shape units
  for shape in tables report; do
    units="$rows"
    if [ "$shape" = tables ]; then
      [ "$units" -gt 100000 ] && units=100000
    else
      [ "$units" -gt 25000 ] && units=25000
    fi
    if [ ! -f "$data/$shape-tables.pages" ]; then
      echo "== generating ${shape} at ${units} table rows" >&2
      python3 bench/pairs/pages-xlsx-gen.py "$shape" "$units" "$data/$shape-tables.md"
      "$sublime" -q convert "$data/$shape-tables.md" "$data/$shape-tables.pages"
    fi
  done
}

# The sum of a ZIP's entry sizes before compression.
pages_xlsx_unzipped() {
  python3 -c 'import sys, zipfile; print(sum(info.file_size for info in zipfile.ZipFile(sys.argv[1]).infolist()))' "$1"
}

run_pair() {
  pages_xlsx_inputs
  echo "== running" >&2
  for shape in tables report; do
    local input="$data/$shape-tables.pages" bytes timing out_bytes seconds
    bytes="$(wc -c < "$input" | tr -d ' ')"
    timing="$(time_cmd ours "$sublime" -q convert "$input" "$data/out-$shape.xlsx")"
    out_bytes="$(pages_xlsx_unzipped "$data/out-$shape.xlsx")"
    seconds="$(seconds_of "$timing")"
    pages_rows pages xlsx "$shape" "$bytes" "$timing"
    row "pages -> xlsx, ${shape} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output) [extra]" "$(mbps "$((bytes + out_bytes))" "$seconds")" "n/a" "n/a"
  done
}
