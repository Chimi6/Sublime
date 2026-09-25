#!/usr/bin/env bash
# XLSX <-> CSV pair. Sourced by bench/run.sh. The workbook is written by
# rust_xlsxwriter (the reference writer) with the csv-json record shape,
# capped at 1,000,000 rows; the CSV is the csv-json generator's file at the
# same cap. References: calamine + csv (read), csv + rust_xlsxwriter (write).

# The sum of a ZIP's entry sizes before compression.
unzipped_bytes() {
  python3 -c 'import sys, zipfile; print(sum(info.file_size for info in zipfile.ZipFile(sys.argv[1]).infolist()))' "$1"
}

run_pair() {
  local units="$rows"
  if [ "$units" -gt 1000000 ]; then
    units=1000000
  fi
  echo "== generating ${units} rows" >&2
  [ -f "$data/big.xlsx" ] || "$bench" xlsx-csv gen-xlsx "$units" "$data/big.xlsx"
  [ -f "$data/medium.csv" ] || "$bench" csv-json gen-csv "$units" "$data/medium.csv"
  local xlsx_bytes csv_bytes xlsx_inflated
  xlsx_bytes="$(wc -c < "$data/big.xlsx" | tr -d ' ')"
  csv_bytes="$(wc -c < "$data/medium.csv" | tr -d ' ')"
  # A workbook is a ZIP: the reader parses the inflated parts, so its
  # throughput row counts them (README, compressed-input paths).
  xlsx_inflated="$(unzipped_bytes "$data/big.xlsx")"

  echo "== running" >&2
  local ours_x2c crates_x2c ours_c2x crates_c2x
  ours_x2c="$(time_cmd ours-xlsx-csv "$sublime" -q convert "$data/big.xlsx" "$data/out-x1.csv")"
  crates_x2c="$(time_cmd crates-xlsx-csv "$bench" xlsx-csv crates-xlsx-csv "$data/big.xlsx" "$data/out-x2.csv")"
  ours_c2x="$(time_cmd ours-csv-xlsx "$sublime" -q convert "$data/medium.csv" "$data/out-x3.xlsx")"
  crates_c2x="$(time_cmd crates-csv-xlsx "$bench" xlsx-csv crates-csv-xlsx "$data/medium.csv" "$data/out-x4.xlsx")"

  # The writer's work is the input plus the uncompressed output it
  # compresses (README, compressed-output paths); ours and the reference
  # write the same rows, so ours' output measures both.
  local out_inflated handled
  out_inflated="$(unzipped_bytes "$data/out-x3.xlsx")"
  handled=$((csv_bytes + out_inflated))

  local ox cx oc cc
  ox="$(seconds_of "$ours_x2c")"; cx="$(seconds_of "$crates_x2c")"
  oc="$(seconds_of "$ours_c2x")"; cc="$(seconds_of "$crates_c2x")"
  row "xlsx -> csv, ${units} rows ($(mb "$xlsx_inflated") MB uncompressed): throughput (MB/s of uncompressed input)" "$(mbps "$xlsx_inflated" "$ox")" "$(mbps "$xlsx_inflated" "$cx") (calamine + csv)" "$(pass "$(echo "$ox <= $cx" | bc -l)")"
  row "xlsx -> csv, ${units} rows ($(mb "$xlsx_bytes") MB on disk): throughput (MB/s of file bytes) [extra]" "$(mbps "$xlsx_bytes" "$ox")" "$(mbps "$xlsx_bytes" "$cx") (calamine + csv)" "n/a"
  row "xlsx -> csv, ${units} rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_x2c")")" "$(rss_mb "$(rss_of "$crates_x2c")") (calamine + csv)" "$(pass "$(echo "$(rss_of "$ours_x2c") <= $(rss_of "$crates_x2c")" | bc -l)")"
  row "csv -> xlsx, ${units} rows ($(mb "$csv_bytes") MB in + $(mb "$out_inflated") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$handled" "$oc")" "$(mbps "$handled" "$cc") (csv + rust_xlsxwriter)" "$(pass "$(echo "$oc <= $cc" | bc -l)")"
  row "csv -> xlsx, ${units} rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_c2x")")" "$(rss_mb "$(rss_of "$crates_c2x")") (csv + rust_xlsxwriter)" "$(pass "$(echo "$(rss_of "$ours_c2x") <= $(rss_of "$crates_c2x")" | bc -l)")"
}
