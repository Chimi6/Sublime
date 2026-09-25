#!/usr/bin/env bash
# TSV <-> JSON pair. Sourced by bench/run.sh. The inputs are the csv-json
# pair's files with the separator swapped (quoted fields keep their commas),
# so the numbers stand beside csv-json's. The reference is the csv crate
# with a tab delimiter and serde_json.

run_pair() {
  echo "== generating ${rows} rows" >&2
  [ -f "$data/big.csv" ] || "$bench" csv-json gen-csv "$rows" "$data/big.csv"
  [ -f "$data/big.json" ] || "$bench" csv-json gen-json "$rows" "$data/big.json"
  [ -f "$data/big.tsv" ] || "$sublime" -q convert "$data/big.csv" "$data/big.tsv"
  local tsv_bytes json_bytes
  tsv_bytes="$(wc -c < "$data/big.tsv" | tr -d ' ')"
  json_bytes="$(wc -c < "$data/big.json" | tr -d ' ')"

  echo "== running" >&2
  local ours_t2j crates_t2j ours_j2t crates_j2t
  ours_t2j="$(time_cmd ours-tsv-json "$sublime" -q convert "$data/big.tsv" "$data/out-t1.json")"
  crates_t2j="$(time_cmd crates-tsv-json "$bench" csv-json crates-tsv-json "$data/big.tsv" "$data/out-t2.json")"
  ours_j2t="$(time_cmd ours-json-tsv "$sublime" -q convert "$data/big.json" "$data/out-t3.tsv")"
  crates_j2t="$(time_cmd crates-json-tsv "$bench" csv-json crates-json-tsv "$data/big.json" "$data/out-t4.tsv")"

  local ot ct oj cj
  ot="$(seconds_of "$ours_t2j")"; ct="$(seconds_of "$crates_t2j")"
  oj="$(seconds_of "$ours_j2t")"; cj="$(seconds_of "$crates_j2t")"
  row "tsv -> json, 10M rows ($(mb "$tsv_bytes") MB): throughput (MB/s of input)" "$(mbps "$tsv_bytes" "$ot")" "$(mbps "$tsv_bytes" "$ct") (csv + serde_json)" "$(pass "$(echo "$ot <= $ct" | bc -l)")"
  row "tsv -> json, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_t2j")")" "$(rss_mb "$(rss_of "$crates_t2j")") (csv + serde_json)" "$(pass "$(echo "$(rss_of "$ours_t2j") <= $(rss_of "$crates_t2j")" | bc -l)")"
  row "json -> tsv, 10M rows ($(mb "$json_bytes") MB): throughput (MB/s of input)" "$(mbps "$json_bytes" "$oj")" "$(mbps "$json_bytes" "$cj") (serde_json + csv)" "$(pass "$(echo "$oj <= $cj" | bc -l)")"
  row "json -> tsv, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_j2t")")" "$(rss_mb "$(rss_of "$crates_j2t")") (serde_json + csv)" "$(pass "$(echo "$(rss_of "$ours_j2t") <= $(rss_of "$crates_j2t")" | bc -l)")"
}
