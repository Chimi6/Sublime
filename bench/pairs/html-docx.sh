#!/usr/bin/env bash
# HTML -> docx. Sourced by bench/run.sh. Inputs are the markdown-html
# generator's two shapes written to HTML by our own writer (no large real
# page can be committed), capped at 20000 units.
# Word is a compressed package, so throughput counts the input plus the
# output's uncompressed bytes (DOCS/benchmarks/README.md). The reference is
# pandoc, timed on the same workload.

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
    local label input bytes ours out_bytes seconds pandoc pseconds
    label="$(shape_label "$shape")"
    input="$data/html-$shape.html"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/html-$shape-ours.docx" --to docx)"
    pandoc="$(time_cmd pandoc pandoc -f html -t docx "$input" -o "$data/html-$shape-pandoc.docx")"
    out_bytes="$(unzip -l "$data/html-$shape-ours.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"
    row "html -> docx, ${label} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$((bytes + out_bytes))" "$seconds")" "$(mbps "$((bytes + out_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "html -> docx, ${label}: throughput (MB/s of input) [extra]" "$(mbps "$bytes" "$seconds")" "recorded" "n/a"
    row "html -> docx, ${label}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$pandoc")") (pandoc)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$pandoc")" | bc -l)")"
  done
}
