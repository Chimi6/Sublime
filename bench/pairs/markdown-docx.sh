#!/usr/bin/env bash
# Markdown -> Word. Sourced by bench/run.sh. Inputs are the markdown-html
# generator's two shapes (markup-dense and prose), capped at 20,000 units
# since the Word output is measured uncompressed. Word is a compressed
# package, so throughput counts the input plus the output's uncompressed
# bytes (DOCS/benchmarks/README.md). The reference is pulldown-cmark
# feeding docx-rs with paragraphs and runs only.

run_pair() {
  local units="$rows"
  if [ "$units" -gt 20000 ]; then
    units=20000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/docx-big.md" ] || "$bench" markdown-html gen "$units" "$data/docx-big.md"
  [ -f "$data/docx-prose.md" ] || "$bench" markdown-html gen-prose "$units" "$data/docx-prose.md"
  echo "== running" >&2
  for shape in big prose; do
    local label input bytes ours crates out_bytes crates_out_bytes seconds cs
    label="markup-dense"; [ "$shape" = prose ] && label="prose"
    input="$data/docx-$shape.md"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/docx-$shape.docx" --to docx)"
    crates="$(time_cmd crates "$bench" markdown-docx crates "$input" "$data/docx-$shape-crates.docx")"
    out_bytes="$(unzip -l "$data/docx-$shape.docx" | tail -1 | awk '{print $1}')"
    crates_out_bytes="$(unzip -l "$data/docx-$shape-crates.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"; cs="$(seconds_of "$crates")"
    row "markdown -> docx, ${label} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$((bytes + out_bytes))" "$seconds")" "$(mbps "$((bytes + crates_out_bytes))" "$cs") (pulldown-cmark + docx-rs, paragraphs and runs only)" "$(pass "$(echo "$seconds <= $cs" | bc -l)")"
    row "markdown -> docx, ${label}: throughput (MB/s of input) [extra]" "$(mbps "$bytes" "$seconds")" "$(mbps "$bytes" "$cs") (reference)" "n/a"
    row "markdown -> docx, ${label}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (reference)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
