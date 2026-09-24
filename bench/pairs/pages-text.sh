#!/usr/bin/env bash
# Pages -> text. Sourced by bench/run.sh. Inputs are the pages-json pair's
# scaled fixtures. There is no peer implementation; the reference is our
# own package round trip (pages -> pages-json) on the same input, which
# decodes every object and writes several times more bytes, so a document
# path that costs more than it is doing avoidable work.

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

run_pair() {
  pages_inputs
  echo "== running" >&2
  for name in styled prose; do
    local input="$data/$name.pages" bytes ours reference
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/$name.txt" --to text)"
    reference="$(time_cmd reference "$sublime" -q convert "$input" "$data/$name.json" --to pages-json)"
    local os rs
    os="$(seconds_of "$ours")"; rs="$(seconds_of "$reference")"
    row "pages -> text throughput, $name (MB/s of the package)" "$(mbps "$bytes" "$os")" "$(mbps "$bytes" "$rs") (pages-json)" "$(pass "$(echo "$os <= $rs" | bc -l)")"
    row "Peak RSS pages -> text, $name (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$reference")") (pages-json)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$reference")" | bc -l)")"
  done
}
