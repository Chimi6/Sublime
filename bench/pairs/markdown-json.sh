#!/usr/bin/env bash
# Markdown -> events as JSON, and back to Markdown. Sourced by bench/run.sh.
# Uses the markdown-html pair's inputs. Forward reference: pulldown-cmark
# events through serde_json. Backward reference: serde_json deserializing
# those events, rendered by pulldown-cmark-to-cmark.

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

  echo "== running forward" >&2
  local ours crates ours_prose crates_prose
  ours="$(time_cmd ours "$sublime" -q convert "$data/big.md" "$data/out1.json" --to markdown-json)"
  crates="$(time_cmd crates "$bench" markdown-json crates "$data/big.md" "$data/out2.json")"
  ours_prose="$(time_cmd ours-prose "$sublime" -q convert "$data/prose.md" "$data/out3.json" --to markdown-json)"
  crates_prose="$(time_cmd crates-prose "$bench" markdown-json crates "$data/prose.md" "$data/out4.json")"
  local os cs ops cps
  os="$(seconds_of "$ours")"; cs="$(seconds_of "$crates")"
  ops="$(seconds_of "$ours_prose")"; cps="$(seconds_of "$crates_prose")"
  row "markdown -> markdown-json, markup-dense ($(mb "$md_bytes") MB): throughput (MB/s of input)" "$(mbps "$md_bytes" "$os")" "$(mbps "$md_bytes" "$cs")" "$(pass "$(echo "$os <= $cs" | bc -l)")"
  row "markdown -> markdown-json, prose ($(mb "$prose_bytes") MB): throughput (MB/s of input)" "$(mbps "$prose_bytes" "$ops")" "$(mbps "$prose_bytes" "$cps")" "$(pass "$(echo "$ops <= $cps" | bc -l)")"
  row "markdown -> markdown-json, markup-dense: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (reference)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"

  echo "== running back" >&2
  local json_bytes back crates_back
  json_bytes="$(wc -c < "$data/out1.json" | tr -d ' ')"
  back="$(time_cmd ours-back "$sublime" -q convert "$data/out1.json" "$data/out5.md" --from markdown-json --to markdown)"
  crates_back="$(time_cmd crates-back "$bench" markdown-json crates-back "$data/out2.json" "$data/out6.md")"
  local bs cbs
  bs="$(seconds_of "$back")"; cbs="$(seconds_of "$crates_back")"
  row "markdown-json -> markdown, markup-dense ($(mb "$json_bytes") MB): throughput (MB/s of input, each side's own JSON)" "$(mbps "$json_bytes" "$bs")" "$(mbps "$(wc -c < "$data/out2.json" | tr -d ' ')" "$cbs")" "$(pass "$(echo "$bs <= $cbs" | bc -l)")"
  row "markdown-json -> markdown, markup-dense: peak memory (MB)" "$(rss_mb "$(rss_of "$back")")" "$(rss_mb "$(rss_of "$crates_back")") (reference)" "$(pass "$(echo "$(rss_of "$back") <= $(rss_of "$crates_back")" | bc -l)")"
}
