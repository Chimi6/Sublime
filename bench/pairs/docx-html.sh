#!/usr/bin/env bash
# Word -> html. Sourced by bench/run.sh. Inputs are the pages-json pair's
# generated documents written to Word by our own writer (no large real
# Word document can be committed). Word is a compressed package, so
# throughput counts the input's uncompressed bytes, what the reader parses
# (DOCS/benchmarks/README.md); the rate per file byte is an extra row.

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

docx_inputs() {
  pages_inputs
  for name in styled prose; do
    [ -f "$data/$name.docx" ] || "$sublime" -q convert "$data/$name.pages" "$data/$name.docx" --to docx
  done
}

# docx_rows <to> <shape> <ours-timing> <pandoc-timing> : the standard rows for
# a Word input, comparing ours against pandoc on the same uncompressed bytes.
docx_rows() {
  local to="$1" name="$2" timing="$3" ptiming="$4"
  local file_bytes in_bytes seconds rss pseconds prss
  file_bytes="$(wc -c < "$data/$name.docx" | tr -d ' ')"
  in_bytes="$(unzip -l "$data/$name.docx" | tail -1 | awk '{print $1}')"
  seconds="$(seconds_of "$timing")"; rss="$(rss_of "$timing")"
  pseconds="$(seconds_of "$ptiming")"; prss="$(rss_of "$ptiming")"
  row "docx -> ${to}, ${name} ($(mb "$in_bytes") MB uncompressed): throughput (MB/s of uncompressed input)" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
  row "docx -> ${to}, ${name} ($(mb "$file_bytes") MB file): throughput (MB/s of file bytes) [extra]" "$(mbps "$file_bytes" "$seconds")" "recorded" "n/a"
  row "docx -> ${to}, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
}

run_pair() {
  docx_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours pandoc
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/$name-from-docx.html" --to html)"
    pandoc="$(time_cmd pandoc pandoc -f docx -t html "$data/$name.docx" -o "$data/$name-pandoc.html")"
    docx_rows html "$name" "$ours" "$pandoc"
  done
}
