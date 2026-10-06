#!/usr/bin/env bash
# RTF <-> docx. Sourced by bench/run.sh. Inputs are the pages-json pair's
# generated documents, written to Word and to RTF by our own writers (no
# large real document can be committed). The reference is pandoc, which
# reads and writes both. Word is a compressed package: docx -> rtf counts
# the input's uncompressed bytes, and rtf -> docx the input plus the
# output's uncompressed bytes (DOCS/benchmarks/README.md).

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

rtf_inputs() {
  pages_inputs
  for name in styled prose; do
    [ -f "$data/$name.docx" ] || "$sublime" -q convert "$data/$name.pages" "$data/$name.docx" --to docx
    [ -f "$data/$name.rtf" ] || "$sublime" -q convert "$data/$name.docx" "$data/$name.rtf" --to rtf
  done
}

run_pair() {
  rtf_inputs
  echo "== running" >&2
  for name in styled prose; do
    local ours pandoc seconds pseconds rss prss in_bytes bytes out_bytes pout_bytes
    # docx -> rtf
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.docx" "$data/out-$name.rtf" --to rtf)"
    pandoc="$(time_cmd pandoc pandoc -f docx -t rtf -s "$data/$name.docx" -o "$data/out-$name-pandoc.rtf")"
    in_bytes="$(unzip -l "$data/$name.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "docx -> rtf, ${name} ($(mb "$in_bytes") MB uncompressed): throughput (MB/s of uncompressed input)" "$(mbps "$in_bytes" "$seconds")" "$(mbps "$in_bytes" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "docx -> rtf, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
    # rtf -> docx
    ours="$(time_cmd ours "$sublime" -q convert "$data/$name.rtf" "$data/out-$name-from-rtf.docx" --to docx)"
    pandoc="$(time_cmd pandoc pandoc -f rtf -t docx "$data/$name.rtf" -o "$data/out-$name-from-rtf-pandoc.docx")"
    bytes="$(wc -c < "$data/$name.rtf" | tr -d ' ')"
    out_bytes="$(unzip -l "$data/out-$name-from-rtf.docx" | tail -1 | awk '{print $1}')"
    pout_bytes="$(unzip -l "$data/out-$name-from-rtf-pandoc.docx" | tail -1 | awk '{print $1}')"
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$pandoc")"; prss="$(rss_of "$pandoc")"
    row "rtf -> docx, ${name} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$((bytes + out_bytes))" "$seconds")" "$(mbps "$((bytes + pout_bytes))" "$pseconds") (pandoc)" "$(pass "$(echo "$seconds <= $pseconds" | bc -l)")"
    row "rtf -> docx, ${name}: throughput (MB/s of input) [extra]" "$(mbps "$bytes" "$seconds")" "$(mbps "$bytes" "$pseconds") (pandoc)" "n/a"
    row "rtf -> docx, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss") (pandoc)" "$(pass "$(echo "$rss <= $prss" | bc -l)")"
  done
}
