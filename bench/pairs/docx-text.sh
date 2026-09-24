#!/usr/bin/env bash
# Word -> text. Sourced by bench/run.sh. Inputs are the pages-json pair's
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

# The reference is docx-rs, a Rust reader, with the paragraph texts joined:
# its parse time bounds what a full Word reader in Rust costs on this input.
run_pair() {
  docx_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours crates
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/$name-from-docx.txt" --to text)"
    crates="$(time_cmd crates "$bench" docx-text crates "$data/$name.docx" "$data/$name-crates.txt")"
    docx_rows text "$name" "$ours"
    local in_bytes
    in_bytes="$(unzip -l "$data/$name.docx" | tail -1 | awk '{print $1}')"
    row "docx -> text, ${name}: throughput (MB/s of uncompressed input) [extra]" "$(mbps "$in_bytes" "$(seconds_of "$ours")")" "$(mbps "$in_bytes" "$(seconds_of "$crates")") (docx-rs, paragraph text only)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "docx -> text, ${name}: peak memory (MB) [extra]" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (docx-rs)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
