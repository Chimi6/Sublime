#!/usr/bin/env bash
# CSV <-> JSON pair. Sourced by bench/run.sh, which provides $sublime, $bench,
# $data, $rows, and the helpers time_cmd, mbps, seconds_of, rss_of, rss_mb,
# pass, row. Defines run_pair, which prints this pair's table rows.

run_pair() {
  echo "== generating ${rows} rows" >&2
  [ -f "$data/big.csv" ] || "$bench" csv-json gen-csv "$rows" "$data/big.csv"
  [ -f "$data/big.json" ] || "$bench" csv-json gen-json "$rows" "$data/big.json"
  local csv_bytes json_bytes
  csv_bytes="$(wc -c < "$data/big.csv" | tr -d ' ')"
  json_bytes="$(wc -c < "$data/big.json" | tr -d ' ')"

  echo "== running" >&2
  local ours_c2j crates_c2j ours_j2c crates_j2c
  ours_c2j="$(time_cmd ours-csv-json "$sublime" -q convert "$data/big.csv" "$data/out1.json")"
  crates_c2j="$(time_cmd crates-csv-json "$bench" csv-json crates-csv-json "$data/big.csv" "$data/out2.json")"
  ours_j2c="$(time_cmd ours-json-csv "$sublime" -q convert "$data/big.json" "$data/out3.csv")"
  crates_j2c="$(time_cmd crates-json-csv "$bench" csv-json crates-json-csv "$data/big.json" "$data/out4.csv")"

  echo "== stdin memory" >&2
  /usr/bin/time -f "%M" -o "$data/rss.txt" sh -c "$sublime -q convert - --from csv --to json < $data/big.csv > $data/out5.json"
  local stdin_rss
  stdin_rss="$(cat "$data/rss.txt")"

  local oc oj cc cj
  oc="$(seconds_of "$ours_c2j")"; cc="$(seconds_of "$crates_c2j")"
  oj="$(seconds_of "$ours_j2c")"; cj="$(seconds_of "$crates_j2c")"
  row "csv -> json, 10M rows ($(mb "$csv_bytes") MB): throughput (MB/s of input)" "$(mbps "$csv_bytes" "$oc")" "$(mbps "$csv_bytes" "$cc")" "$(pass "$(echo "$oc <= $cc" | bc -l)")"
  row "json -> csv, 10M rows ($(mb "$json_bytes") MB): throughput (MB/s of input)" "$(mbps "$json_bytes" "$oj")" "$(mbps "$json_bytes" "$cj")" "$(pass "$(echo "$oj <= $cj" | bc -l)")"
  row "csv -> json, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_c2j")")" "< 16" "$(pass "$(echo "$(rss_of "$ours_c2j") < 16384" | bc -l)")"
  row "json -> csv, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_j2c")")" "< 16" "$(pass "$(echo "$(rss_of "$ours_j2c") < 16384" | bc -l)")"
  row "csv -> json, 10M rows from stdin: peak memory (MB)" "$(rss_mb "$stdin_rss")" "< 16" "$(pass "$(echo "$stdin_rss < 16384" | bc -l)")"
}
