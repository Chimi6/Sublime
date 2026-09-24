#!/usr/bin/env bash
# HTML -> text. Sourced by bench/run.sh. Inputs are the markdown-html
# generator's two shapes written to HTML by our own writer (no large real
# page can be committed), capped at 20,000 units.
# No peer converts HTML to text in Rust without a browser engine; the
# reference column holds the goals the document paths share (50 MB/s of
# input, 64 MB peak).

html_inputs() {
  local units="$rows"
  if [ "$units" -gt 20000 ]; then
    units=20000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/html-big.md" ] || "$bench" markdown-html gen "$units" "$data/html-big.md"
  [ -f "$data/html-prose.md" ] || "$bench" markdown-html gen-prose "$units" "$data/html-prose.md"
  for shape in big prose; do
    [ -f "$data/html-$shape.html" ] || "$sublime" -q convert "$data/html-$shape.md" "$data/html-$shape.html" --to html
  done
}

shape_label() { if [ "$1" = prose ]; then echo prose; else echo markup-dense; fi; }

run_pair() {
  html_inputs
  echo "== running" >&2
  for shape in big prose; do
    local label input bytes ours
    label="$(shape_label "$shape")"
    input="$data/html-$shape.html"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/html-$shape-ours.txt" --to text)"
    row "html -> text, ${label} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "goal: 50" "$(pass "$(echo "$(mbps "$bytes" "$(seconds_of "$ours")") >= 50" | bc -l)")"
    row "html -> text, ${label}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "goal: <= 64.0" "$(pass "$(echo "$(rss_of "$ours") <= 65536" | bc -l)")"
  done
}
