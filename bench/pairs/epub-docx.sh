#!/usr/bin/env bash
# EPUB <-> docx. Sourced by bench/run.sh. Inputs are the pages-json pair's
# generated documents, written to Word and to EPUB by our own writers (no
# large real document can be committed). The reference is pandoc, which
# reads and writes both. Both are compressed packages: each direction counts
# the input's plus the output's uncompressed bytes, the parse and the
# compressor's work (DOCS/benchmarks/README.md).

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

epub_inputs() {
  pages_inputs
  for name in styled prose; do
    [ -f "$data/$name.docx" ] || "$sublime" -q convert "$data/$name.pages" "$data/$name.docx" --to docx
    [ -f "$data/$name.epub" ] || "$sublime" -q convert "$data/$name.docx" "$data/$name.epub" --to epub
  done
}

unpacked() {
  unzip -l "$1" | tail -1 | awk '{print $1}'
}

run_pair() {
  epub_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours pandoc seconds pseconds rss prss in_bytes out_bytes pout_bytes
    # docx -> epub
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/out-$name.epub" --to epub)"
    pandoc="$(time_cmd pandoc pandoc -f docx -t epub3 "$data/$name.docx" -o "$data/out-$name-pandoc.epub")"
    in_bytes="$(unpacked "$data/$name.docx")"
    out_bytes="$(unpacked "$data/out-$name.epub")"
    pout_bytes="$(unpacked "$data/out-$name-pandoc.epub")"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "docx -> epub, ${name} ($(mb "$in_bytes") MB in + $(mb "$out_bytes") MB out, uncompressed): throughput (MB/s of input plus output)" "$(mbps "$((in_bytes + out_bytes))" "$seconds")" "$(mbps "$((in_bytes + pout_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "docx -> epub, ${name}: throughput (MB/s of uncompressed input) [extra]" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "n/a"
    row "docx -> epub, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
    # epub -> docx
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.epub" "$data/out-$name-from-epub.docx" --to docx)"
    pandoc="$(time_cmd pandoc pandoc -f epub -t docx "$data/$name.epub" -o "$data/out-$name-from-epub-pandoc.docx")"
    in_bytes="$(unpacked "$data/$name.epub")"
    out_bytes="$(unpacked "$data/out-$name-from-epub.docx")"
    pout_bytes="$(unpacked "$data/out-$name-from-epub-pandoc.docx")"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "epub -> docx, ${name} ($(mb "$in_bytes") MB in + $(mb "$out_bytes") MB out, uncompressed): throughput (MB/s of input plus output)" "$(mbps "$((in_bytes + out_bytes))" "$seconds")" "$(mbps "$((in_bytes + pout_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "epub -> docx, ${name}: throughput (MB/s of uncompressed input) [extra]" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "n/a"
    row "epub -> docx, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
  done
}
