#!/usr/bin/env bash
# Markdown -> HTML pair. Sourced by bench/run.sh. $rows is the number of
# generated units (about 1.1 KB each); the default 100000 gives ~77 MB.
# Two inputs: one with footnotes and reference links in every twentieth
# unit, one without any, because footnote bookkeeping can dominate a
# reference implementation and hide the parser comparison.

run_pair() {
  local units="$rows"
  if [ "$units" -gt 200000 ]; then
    units=100000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/big.md" ] || "$bench" markdown-html gen "$units" "$data/big.md"
  [ -f "$data/plain.md" ] || "$bench" markdown-html gen-plain "$units" "$data/plain.md"
  local md_bytes plain_bytes
  md_bytes="$(wc -c < "$data/big.md" | tr -d ' ')"
  plain_bytes="$(wc -c < "$data/plain.md" | tr -d ' ')"

  echo "== running" >&2
  local ours crates ours_plain crates_plain
  ours="$(time_cmd ours "$sublime" -q convert "$data/big.md" "$data/out1.html")"
  crates="$(time_cmd crates "$bench" markdown-html crates "$data/big.md" "$data/out2.html")"
  ours_plain="$(time_cmd ours-plain "$sublime" -q convert "$data/plain.md" "$data/out3.html")"
  crates_plain="$(time_cmd crates-plain "$bench" markdown-html crates "$data/plain.md" "$data/out4.html")"

  local os cs ops cps
  os="$(seconds_of "$ours")"; cs="$(seconds_of "$crates")"
  ops="$(seconds_of "$ours_plain")"; cps="$(seconds_of "$crates_plain")"
  row "Markdown -> HTML throughput, no footnotes (MB/s)" "$(mbps "$plain_bytes" "$ops")" "$(mbps "$plain_bytes" "$cps")" "$(pass "$(echo "$ops <= $cps" | bc -l)")"
  row "Markdown -> HTML throughput, footnotes in 1/20 units (MB/s)" "$(mbps "$md_bytes" "$os")" "$(mbps "$md_bytes" "$cs")" "$(pass "$(echo "$os <= $cs" | bc -l)")"
  row "Peak RSS Markdown -> HTML, no footnotes (MB)" "$(rss_mb "$(rss_of "$ours_plain")")" "$(rss_mb "$(rss_of "$crates_plain")") (reference)" "$(pass "$(echo "$(rss_of "$ours_plain") <= $(rss_of "$crates_plain")" | bc -l)")"
  row "Peak RSS Markdown -> HTML, footnotes in 1/20 units (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (reference)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
}
