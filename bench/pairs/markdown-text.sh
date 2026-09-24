#!/usr/bin/env bash
# Markdown -> plain text. Sourced by bench/run.sh. Uses the markdown-html
# pair's inputs (markup-dense and prose). The reference is a bare text dump
# of pulldown-cmark events, which does less than our renderer.

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
  ours="$(time_cmd ours "$sublime" -q convert "$data/big.md" "$data/out1.txt")"
  crates="$(time_cmd crates "$bench" markdown-text crates "$data/big.md" "$data/out2.txt")"
  ours_prose="$(time_cmd ours-prose "$sublime" -q convert "$data/prose.md" "$data/out3.txt")"
  crates_prose="$(time_cmd crates-prose "$bench" markdown-text crates "$data/prose.md" "$data/out4.txt")"

  local os cs ops cps
  os="$(seconds_of "$ours")"; cs="$(seconds_of "$crates")"
  ops="$(seconds_of "$ours_prose")"; cps="$(seconds_of "$crates_prose")"
  row "markdown -> text, markup-dense ($(mb "$md_bytes") MB): throughput (MB/s of input)" "$(mbps "$md_bytes" "$os")" "$(mbps "$md_bytes" "$cs")" "$(pass "$(echo "$os <= $cs" | bc -l)")"
  row "markdown -> text, prose ($(mb "$prose_bytes") MB): throughput (MB/s of input)" "$(mbps "$prose_bytes" "$ops")" "$(mbps "$prose_bytes" "$cps")" "$(pass "$(echo "$ops <= $cps" | bc -l)")"
  row "markdown -> text, markup-dense: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (reference)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  row "markdown -> text, prose: peak memory (MB)" "$(rss_mb "$(rss_of "$ours_prose")")" "$(rss_mb "$(rss_of "$crates_prose")") (reference)" "$(pass "$(echo "$(rss_of "$ours_prose") <= $(rss_of "$crates_prose")" | bc -l)")"
}
