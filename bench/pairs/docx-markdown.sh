#!/usr/bin/env bash
# Word -> markdown. Sourced by bench/run.sh. Inputs are the pages-json pair's
# generated documents written to Word by our own writer (no large real
# Word document can be committed). Word is a compressed package, so
# throughput counts the input's uncompressed bytes, what the reader parses
# (DOCS/benchmarks/README.md); the rate per file byte is an extra row.

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

docx_inputs() {
  pages_inputs
  for name in styled prose; do
    [ -f "$data/$name.docx" ] || "$sublime" -q convert "$data/$name.pages" "$data/$name.docx" --to docx
  done
}

# docx_rows <to> <shape> <timing> : the standard rows for a Word input.
docx_rows() {
  local to="$1" name="$2" timing="$3"
  local file_bytes in_bytes seconds rss
  file_bytes="$(wc -c < "$data/$name.docx" | tr -d ' ')"
  in_bytes="$(unzip -l "$data/$name.docx" | tail -1 | awk '{print $1}')"
  seconds="$(seconds_of "$timing")"; rss="$(rss_of "$timing")"
  row "docx -> ${to}, ${name} ($(mb "$in_bytes") MB uncompressed): throughput (MB/s of uncompressed input)" "$(mbps "$in_bytes" "$seconds")" "goal: ${pages_goal_mbps}" "$(pass "$(echo "$(mbps "$in_bytes" "$seconds") >= $pages_goal_mbps" | bc -l)")"
  row "docx -> ${to}, ${name} ($(mb "$file_bytes") MB file): throughput (MB/s of file bytes) [extra]" "$(mbps "$file_bytes" "$seconds")" "recorded" "n/a"
  row "docx -> ${to}, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "goal: <= $(rss_mb "$pages_goal_rss_kb")" "$(pass "$(echo "$rss <= $pages_goal_rss_kb" | bc -l)")"
}

run_pair() {
  docx_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/$name-from-docx.md" --to markdown)"
    docx_rows markdown "$name" "$ours"
  done
}
