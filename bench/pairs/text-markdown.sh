#!/usr/bin/env bash
# Text -> Markdown. Sourced by bench/run.sh. Two shapes: prose (the
# markdown-html generator's prose written to plain text by our own writer)
# and dense (short lines full of characters Markdown would read as markup,
# from the harness generator). No peer converts text to Markdown; the
# reference column holds the goals the document paths share (50 MB/s of
# input, 64 MB peak).

run_pair() {
  local units="$rows"
  if [ "$units" -gt 100000 ]; then
    units=100000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/text-prose.md" ] || "$bench" markdown-html gen-prose "$units" "$data/text-prose.md"
  [ -f "$data/text-prose.txt" ] || "$sublime" -q convert "$data/text-prose.md" "$data/text-prose.txt" --to text
  [ -f "$data/text-dense.txt" ] || "$bench" text-markdown gen-lines "$units" "$data/text-dense.txt"
  echo "== running" >&2
  for shape in dense prose; do
    local input bytes ours
    input="$data/text-$shape.txt"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/text-$shape-ours.md" --to markdown)"
    row "text -> markdown, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "goal: 50" "$(pass "$(echo "$(mbps "$bytes" "$(seconds_of "$ours")") >= 50" | bc -l)")"
    row "text -> markdown, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "goal: <= 64.0" "$(pass "$(echo "$(rss_of "$ours") <= 65536" | bc -l)")"
  done
}
