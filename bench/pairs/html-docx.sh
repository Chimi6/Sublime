#!/usr/bin/env bash
# HTML -> docx. Sourced by bench/run.sh. Inputs are the markdown-html
# generator's two shapes written to HTML by our own writer (no large real
# page can be committed), capped at 20000 units.
# Word is a compressed package, so throughput counts the input plus the
# output's uncompressed bytes (DOCS/benchmarks/README.md). No peer converts
# HTML to Word in Rust; the reference column holds the goals the document
# paths share (50 MB/s, 64 MB peak).

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
    local label input bytes ours out_bytes seconds
    label="$(shape_label "$shape")"
    input="$data/html-$shape.html"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/html-$shape-ours.docx" --to docx)"
    out_bytes="$(unzip -l "$data/html-$shape-ours.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"
    row "html -> docx, ${label} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$((bytes + out_bytes))" "$seconds")" "goal: 50" "$(pass "$(echo "$(mbps "$((bytes + out_bytes))" "$seconds") >= 50" | bc -l)")"
    row "html -> docx, ${label}: throughput (MB/s of input) [extra]" "$(mbps "$bytes" "$seconds")" "recorded" "n/a"
    row "html -> docx, ${label}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "goal: <= 64.0" "$(pass "$(echo "$(rss_of "$ours") <= 65536" | bc -l)")"
  done
}
