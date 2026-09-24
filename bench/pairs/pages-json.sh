#!/usr/bin/env bash
# Pages <-> pages-json. Sourced by bench/run.sh. Inputs are fixtures scaled
# by the harness generator: `styled` repeats text-styles.pages (short
# paragraphs, many character style runs) and `prose` repeats
# paragraphs.pages (long paragraphs, few runs). No other tool reads the
# modern Pages format, so the reference column holds the goals shared by
# every Pages pair: 50 MB/s of input and 64 MB peak (DOCS/benchmarks/pages-json.md).

pages_goal_mbps=50
pages_goal_rss_kb=65536

pages_inputs() {
  local styled_units="$rows" prose_units="$rows"
  [ "$styled_units" -gt 5000 ] && styled_units=5000
  [ "$prose_units" -gt 2000 ] && prose_units=2000
  echo "== generating styled x${styled_units}, prose x${prose_units}" >&2
  [ -f "$data/styled.pages" ] || "$bench" pages-json gen tests/fixtures/pages/text-styles.pages "$styled_units" "$data/styled.pages"
  [ -f "$data/prose.pages" ] || "$bench" pages-json gen tests/fixtures/pages/paragraphs.pages "$prose_units" "$data/prose.pages"
}

# pages_rows <from> <to> <shape> <input bytes> <timing> : the standard
# throughput and memory rows against the shared goals.
pages_rows() {
  local from="$1" to="$2" shape="$3" bytes="$4" timing="$5"
  local seconds rss
  seconds="$(seconds_of "$timing")"; rss="$(rss_of "$timing")"
  row "${from} -> ${to}, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$seconds")" "goal: ${pages_goal_mbps}" "$(pass "$(echo "$(mbps "$bytes" "$seconds") >= $pages_goal_mbps" | bc -l)")"
  row "${from} -> ${to}, ${shape}: peak memory (MB)" "$(rss_mb "$rss")" "goal: <= $(rss_mb "$pages_goal_rss_kb")" "$(pass "$(echo "$rss <= $pages_goal_rss_kb" | bc -l)")"
}

run_pair() {
  pages_inputs
  echo "== running" >&2
  for name in styled prose; do
    local input="$data/$name.pages" pages_bytes json_bytes forward backward
    pages_bytes="$(wc -c < "$input" | tr -d ' ')"
    forward="$(time_cmd to-json "$sublime" -q convert "$input" "$data/$name.json" --to pages-json)"
    backward="$(time_cmd back "$sublime" -q convert "$data/$name.json" "$data/$name-back.pages" --from pages-json --to pages)"
    json_bytes="$(wc -c < "$data/$name.json" | tr -d ' ')"
    pages_rows pages pages-json "$name" "$pages_bytes" "$forward"
    pages_rows pages-json pages "$name" "$json_bytes" "$backward"
  done
}
