#!/usr/bin/env bash
# CSV <-> Markdown table pair. Sourced by bench/run.sh. The CSV is the
# csv-json generator's file capped at 1,000,000 rows; the Markdown is our
# own table of it. References: Miller (`mlr --icsv --omd`, the real tool
# that writes Markdown tables from CSV) for CSV to Markdown, with the csv
# crate plus a hand-written table printer as an extra row for context, and
# pulldown-cmark's events into the csv crate for Markdown to CSV.
# Homebrew's bin is added to the PATH when present, for Miller on Linux.

if [ -d /home/linuxbrew/.linuxbrew/bin ]; then
  export PATH="/home/linuxbrew/.linuxbrew/bin:$PATH"
fi

run_pair() {
  local units="$rows"
  if [ "$units" -gt 1000000 ]; then
    units=1000000
  fi
  echo "== generating ${units} rows" >&2
  [ -f "$data/medium.csv" ] || "$bench" csv-json gen-csv "$units" "$data/medium.csv"
  [ -f "$data/medium-table.md" ] || "$sublime" -q convert "$data/medium.csv" "$data/medium-table.md"
  local csv_bytes md_bytes
  csv_bytes="$(wc -c < "$data/medium.csv" | tr -d ' ')"
  md_bytes="$(wc -c < "$data/medium-table.md" | tr -d ' ')"

  echo "== running" >&2
  local ours_c2m crates_c2m miller_c2m ours_m2c crates_m2c
  ours_c2m="$(time_cmd ours-csv-markdown "$sublime" -q convert "$data/medium.csv" "$data/out-m1.md")"
  crates_c2m="$(time_cmd crates-csv-markdown "$bench" csv-markdown crates-csv-markdown "$data/medium.csv" "$data/out-m2.md")"
  miller_c2m="$(time_cmd miller-csv-markdown sh -c "mlr --icsv --omd cat $data/medium.csv > $data/out-m5.md")"
  ours_m2c="$(time_cmd ours-markdown-csv "$sublime" -q convert "$data/medium-table.md" "$data/out-m3.csv")"
  crates_m2c="$(time_cmd crates-markdown-csv "$bench" csv-markdown crates-markdown-csv "$data/medium-table.md" "$data/out-m4.csv")"

  local oc cc mc om cm
  oc="$(seconds_of "$ours_c2m")"; cc="$(seconds_of "$crates_c2m")"; mc="$(seconds_of "$miller_c2m")"
  om="$(seconds_of "$ours_m2c")"; cm="$(seconds_of "$crates_m2c")"
  row "csv -> markdown, ${units} rows ($(mb "$csv_bytes") MB): throughput (MB/s of input)" "$(mbps "$csv_bytes" "$oc")" "$(mbps "$csv_bytes" "$mc") (Miller)" "$(pass "$(echo "$oc <= $mc" | bc -l)")"
  row "csv -> markdown, ${units} rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_c2m")")" "$(rss_mb "$(rss_of "$miller_c2m")") (Miller)" "$(pass "$(echo "$(rss_of "$ours_c2m") <= $(rss_of "$miller_c2m")" | bc -l)")"
  row "csv -> markdown, ${units} rows: throughput against a bespoke printer in the harness (MB/s of input) [extra]" "$(mbps "$csv_bytes" "$oc")" "$(mbps "$csv_bytes" "$cc") (csv + table printer)" "n/a"
  row "markdown -> csv, ${units} rows ($(mb "$md_bytes") MB): throughput (MB/s of input)" "$(mbps "$md_bytes" "$om")" "$(mbps "$md_bytes" "$cm") (pulldown-cmark + csv)" "$(pass "$(echo "$om <= $cm" | bc -l)")"
  row "markdown -> csv, ${units} rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_m2c")")" "$(rss_mb "$(rss_of "$crates_m2c")") (pulldown-cmark + csv)" "$(pass "$(echo "$(rss_of "$ours_m2c") <= $(rss_of "$crates_m2c")" | bc -l)")"
}
