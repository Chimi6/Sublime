#!/usr/bin/env bash
# Numbers -> CSV. Sourced by bench/run.sh (and by numbers-xlsx.sh for its
# inputs). Inputs:
#   real:   issue-50.numbers from numbers-parser's test data (MIT), a
#           65,553-row table Numbers itself saved, fetched at a pinned commit;
#   dense:  one table of records (eight mixed columns), written by
#           numbers-parser, capped at 200,000 rows;
#   sheets: twenty sheets of three tables each, the same records spread
#           across them.
# References: numbers-parser (with Python's csv module), the library most
# tools read Numbers through, and LibreOffice (libetonyek), the other
# converter. LibreOffice reads only the last 256-row tile of a table
# numbers-parser wrote, so it is timed on the real document only. Every
# tool writes a CSV per table. A Numbers document is a ZIP of stored
# entries, so its bytes on disk are what the readers parse.

numbers_parser_commit=9a4a927bce03b84306f4f7dd102c2190f53059ee
table_header="| Target | Ours | numbers-parser | LibreOffice | Result |"
table_sep="|---|---|---|---|---|"
lo_filter='csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,false,false,false,-1'

numbers_inputs() {
  local venv="$data/numbers-venv"
  [ -x "$venv/bin/python" ] || python3 -m venv "$venv"
  "$venv/bin/python" -c 'import numbers_parser, openpyxl' 2>/dev/null || "$venv/bin/pip" -q install numbers-parser openpyxl
  python="$venv/bin/python"
  local units="$rows"
  [ "$units" -gt 200000 ] && units=200000
  echo "== generating dense and sheets at ${units} rows" >&2
  [ -f "$data/real.numbers" ] || curl -sSfL -o "$data/real.numbers" \
    "https://raw.githubusercontent.com/masaccio/numbers-parser/${numbers_parser_commit}/tests/data/issue-50.numbers"
  [ -f "$data/dense.numbers" ] || "$python" bench/pairs/numbers-gen.py dense "$units" "$data/dense.numbers"
  [ -f "$data/sheets.numbers" ] || "$python" bench/pairs/numbers-gen.py sheets "$units" "$data/sheets.numbers"
}

run_pair() {
  numbers_inputs
  echo "== running" >&2
  for name in real dense sheets; do
    local input="$data/$name.numbers" bytes ours parser office seconds pseconds oseconds rss prss orss lo_mbps lo_rss verdict
    bytes="$(wc -c < "$input" | tr -d ' ')"
    rm -rf "$data/out-$name" "$data/np-$name" "$data/lo-$name"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/out-$name.csv")"
    parser="$(time_cmd numbers-parser "$python" bench/pairs/numbers-parser-csv.py "$input" "$data/np-$name")"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$parser")"; prss="$(rss_of "$parser")"
    lo_mbps="n/a"; lo_rss="n/a"
    verdict="$(echo "$seconds <= $pseconds" | bc -l)"
    if [ "$name" = real ] && command -v soffice >/dev/null; then
      # soffice reports each sheet on standard output; the timing line must
      # be the only output.
      office="$(time_cmd libreoffice sh -c 'soffice --headless --convert-to "$1" --outdir "$2" "$3" >/dev/null 2>&1' sh "$lo_filter" "$data/lo-$name" "$input")"
      oseconds="$(seconds_of "$office")"; orss="$(rss_of "$office")"
      lo_mbps="$(mbps "$bytes" "$oseconds")"; lo_rss="$(rss_mb "$orss")"
      verdict="$(echo "$seconds <= $pseconds && $seconds <= $oseconds" | bc -l)"
    fi
    row "numbers -> csv, ${name} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$seconds")" "$(mbps "$bytes" "$pseconds")" "$lo_mbps" "$(pass "$verdict")"
    verdict="$(echo "$rss <= $prss" | bc -l)"
    [ "$lo_rss" != n/a ] && verdict="$(echo "$rss <= $prss && $rss <= $orss" | bc -l)"
    row "numbers -> csv, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss")" "$lo_rss" "$(pass "$verdict")"
  done
}
