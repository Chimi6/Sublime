#!/usr/bin/env bash
# Markdown -> HTML pair. Sourced by bench/run.sh. $rows is the number of
# generated units; the default 10000000 is capped at 100000 units.
# Two inputs: a markup-dense document (headings, lists, code, quotes,
# tables, links, with footnotes and reference definitions in every
# twentieth unit, about 1.1 KB per unit) and a prose document (long
# paragraphs of plain sentences with the odd emphasis, link, or code span,
# about 0.6 KB per unit), since most real documents are mostly prose.

run_pair() {
  local units="$rows"
  if [ "$units" -gt 100000 ]; then
    units=100000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/big.md" ] || "$bench" markdown-html gen "$units" "$data/big.md"
  [ -f "$data/prose.md" ] || "$bench" markdown-html gen-prose "$units" "$data/prose.md"
  local md_bytes prose_bytes
  md_bytes="$(wc -c < "$data/big.md" | tr -d ' ')"
  prose_bytes="$(wc -c < "$data/prose.md" | tr -d ' ')"

  echo "== running" >&2
  local ours crates ours_prose crates_prose
  ours="$(time_cmd ours "$sublime" -q convert "$data/big.md" "$data/out1.html")"
  crates="$(time_cmd crates "$bench" markdown-html crates "$data/big.md" "$data/out2.html")"
  ours_prose="$(time_cmd ours-prose "$sublime" -q convert "$data/prose.md" "$data/out3.html")"
  crates_prose="$(time_cmd crates-prose "$bench" markdown-html crates "$data/prose.md" "$data/out4.html")"

  local os cs ops cps
  os="$(seconds_of "$ours")"; cs="$(seconds_of "$crates")"
  ops="$(seconds_of "$ours_prose")"; cps="$(seconds_of "$crates_prose")"
  row "markdown -> html, markup-dense ($(mb "$md_bytes") MB): throughput (MB/s of input)" "$(mbps "$md_bytes" "$os")" "$(mbps "$md_bytes" "$cs")" "$(pass "$(echo "$os <= $cs" | bc -l)")"
  row "markdown -> html, prose ($(mb "$prose_bytes") MB): throughput (MB/s of input)" "$(mbps "$prose_bytes" "$ops")" "$(mbps "$prose_bytes" "$cps")" "$(pass "$(echo "$ops <= $cps" | bc -l)")"
  row "markdown -> html, markup-dense: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (reference)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  row "markdown -> html, prose: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_prose")")" "$(rss_mb "$(rss_of "$crates_prose")") (reference)" "$(pass "$(echo "$(rss_of "$ours_prose") <= $(rss_of "$crates_prose")" | bc -l)")"
}
