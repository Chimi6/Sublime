#!/usr/bin/env bash
# ODT <-> docx. Sourced by bench/run.sh. Inputs are the pages-json pair's
# generated documents, written to Word and to ODT by our own writers (no
# large real document can be committed). The reference is pandoc, which
# reads and writes both. Both are compressed packages: each direction counts
# the input's plus the output's uncompressed bytes, the parse and the
# compressor's work (DOCS/benchmarks/README.md).

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

odt_inputs() {
  pages_inputs
  for name in styled prose; do
    [ -f "$data/$name.docx" ] || "$sublime" -q convert "$data/$name.pages" "$data/$name.docx" --to docx
    [ -f "$data/$name.odt" ] || "$sublime" -q convert "$data/$name.docx" "$data/$name.odt" --to odt
  done
}

unpacked() {
  unzip -l "$1" | tail -1 | awk '{print $1}'
}

run_pair() {
  odt_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours pandoc seconds pseconds rss prss in_bytes out_bytes pout_bytes
    # docx -> odt
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/out-$name.odt" --to odt)"
    pandoc="$(time_cmd pandoc pandoc -f docx -t odt "$data/$name.docx" -o "$data/out-$name-pandoc.odt")"
    in_bytes="$(unpacked "$data/$name.docx")"
    out_bytes="$(unpacked "$data/out-$name.odt")"
    pout_bytes="$(unpacked "$data/out-$name-pandoc.odt")"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "docx -> odt, ${name} ($(mb "$in_bytes") MB in + $(mb "$out_bytes") MB out, uncompressed): throughput (MB/s of input plus output)" "$(mbps "$((in_bytes + out_bytes))" "$seconds")" "$(mbps "$((in_bytes + pout_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "docx -> odt, ${name}: throughput (MB/s of uncompressed input) [extra]" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "n/a"
    row "docx -> odt, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
    # odt -> docx
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.odt" "$data/out-$name-from-odt.docx" --to docx)"
    pandoc="$(time_cmd pandoc pandoc -f odt -t docx "$data/$name.odt" -o "$data/out-$name-from-odt-pandoc.docx")"
    in_bytes="$(unpacked "$data/$name.odt")"
    out_bytes="$(unpacked "$data/out-$name-from-odt.docx")"
    pout_bytes="$(unpacked "$data/out-$name-from-odt-pandoc.docx")"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "odt -> docx, ${name} ($(mb "$in_bytes") MB in + $(mb "$out_bytes") MB out, uncompressed): throughput (MB/s of input plus output)" "$(mbps "$((in_bytes + out_bytes))" "$seconds")" "$(mbps "$((in_bytes + pout_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "odt -> docx, ${name}: throughput (MB/s of uncompressed input) [extra]" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "n/a"
    row "odt -> docx, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
  done
}
