#!/usr/bin/env bash
# Pages -> docx. Sourced by bench/run.sh. Inputs and goals come from the
# pages-json pair: no other tool reads the modern Pages format, so the
# reference column holds the goals every Pages pair shares. Word is a
# compressed package, so throughput counts the input plus the output's
# uncompressed bytes (DOCS/benchmarks/README.md).

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

run_pair() {
  pages_inputs
  echo "== running" >&2
  for name in styled prose; do
    local input="$data/$name.pages" bytes ours
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/$name.docx" --to docx)"
    local out_bytes seconds rss
    out_bytes="$(unzip -l "$data/$name.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    row "pages -> docx, ${name} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$((bytes + out_bytes))" "$seconds")" "goal: ${pages_goal_mbps}" "$(pass "$(echo "$(mbps "$((bytes + out_bytes))" "$seconds") >= $pages_goal_mbps" | bc -l)")"
    row "pages -> docx, ${name}: throughput (MB/s of input) [extra]" "$(mbps "$bytes" "$seconds")" "recorded" "n/a"
    row "pages -> docx, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "goal: <= $(rss_mb "$pages_goal_rss_kb")" "$(pass "$(echo "$rss <= $pages_goal_rss_kb" | bc -l)")"
  done
}
