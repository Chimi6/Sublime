#!/usr/bin/env bash
# HTML -> text. Sourced by bench/run.sh. Inputs are the markdown-html
# generator's two shapes written to HTML by our own writer (no large real
# page can be committed), capped at 20,000 units.
# Two references: html2text, a pure-Rust HTML to text renderer built on
# html5ever (the tight peer and the pass gate), and pandoc (a general-purpose
# document converter) as a real-world tool for context. Two reference columns.

table_header="| Target | Ours | html2text | pandoc | Result |"
table_sep="|---|---|---|---|---|"

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
    local label input bytes ours crates pandoc
    label="$(shape_label "$shape")"
    input="$data/html-$shape.html"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/html-$shape-ours.txt" --to text)"
    crates="$(time_cmd crates "$bench" html-text crates "$input" "$data/html-$shape-crates.txt")"
    pandoc="$(time_cmd pandoc pandoc -f html -t plain "$input" -o "$data/html-$shape-pandoc.txt")"
    row "html -> text, ${label} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")")" "$(mbps "$bytes" "$(seconds_of "$pandoc")")" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "html -> text, ${label}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")")" "$(rss_mb "$(rss_of "$pandoc")")" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
