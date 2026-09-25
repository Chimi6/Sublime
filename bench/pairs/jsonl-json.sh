#!/usr/bin/env bash
# JSON Lines <-> JSON pair. Sourced by bench/run.sh. One shape: the csv-json
# records with a nested array, one per line, and the same as one array.
# The reference is serde_json's stream deserializer and serializer.

run_pair() {
  echo "== generating ${rows} rows" >&2
  [ -f "$data/big.jsonl" ] || "$bench" jsonl-json gen-jsonl "$rows" "$data/big.jsonl"
  [ -f "$data/big-lines.json" ] || "$sublime" -q convert "$data/big.jsonl" "$data/big-lines.json"
  local jsonl_bytes json_bytes
  jsonl_bytes="$(wc -c < "$data/big.jsonl" | tr -d ' ')"
  json_bytes="$(wc -c < "$data/big-lines.json" | tr -d ' ')"

  echo "== running" >&2
  local ours_l2j crates_l2j ours_j2l crates_j2l
  ours_l2j="$(time_cmd ours-jsonl-json "$sublime" -q convert "$data/big.jsonl" "$data/out-l1.json")"
  crates_l2j="$(time_cmd crates-jsonl-json "$bench" jsonl-json crates-jsonl-json "$data/big.jsonl" "$data/out-l2.json")"
  ours_j2l="$(time_cmd ours-json-jsonl "$sublime" -q convert "$data/big-lines.json" "$data/out-l3.jsonl")"
  crates_j2l="$(time_cmd crates-json-jsonl "$bench" jsonl-json crates-json-jsonl "$data/big-lines.json" "$data/out-l4.jsonl")"

  local ol cl oj cj
  ol="$(seconds_of "$ours_l2j")"; cl="$(seconds_of "$crates_l2j")"
  oj="$(seconds_of "$ours_j2l")"; cj="$(seconds_of "$crates_j2l")"
  row "jsonl -> json, 10M rows ($(mb "$jsonl_bytes") MB): throughput (MB/s of input)" "$(mbps "$jsonl_bytes" "$ol")" "$(mbps "$jsonl_bytes" "$cl") (serde_json)" "$(pass "$(echo "$ol <= $cl" | bc -l)")"
  row "jsonl -> json, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_l2j")")" "$(rss_mb "$(rss_of "$crates_l2j")") (serde_json)" "$(pass "$(echo "$(rss_of "$ours_l2j") <= $(rss_of "$crates_l2j")" | bc -l)")"
  row "json -> jsonl, 10M rows ($(mb "$json_bytes") MB): throughput (MB/s of input)" "$(mbps "$json_bytes" "$oj")" "$(mbps "$json_bytes" "$cj") (serde_json)" "$(pass "$(echo "$oj <= $cj" | bc -l)")"
  row "json -> jsonl, 10M rows: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_j2l")")" "$(rss_mb "$(rss_of "$crates_j2l")") (serde_json)" "$(pass "$(echo "$(rss_of "$ours_j2l") <= $(rss_of "$crates_j2l")" | bc -l)")"
}
