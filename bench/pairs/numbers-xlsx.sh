#!/usr/bin/env bash
# Numbers -> xlsx. Sourced by bench/run.sh. The numbers-csv pair's inputs
# (real, dense, sheets) and references: numbers-parser with openpyxl in
# write-only (streaming) mode, a worksheet per table
# (bench/pairs/numbers-parser-xlsx.py), and LibreOffice on the real
# document only. Excel output is compressed, so throughput counts the
# input plus the output's uncompressed bytes (DOCS/benchmarks/README.md);
# every tool writes the same cells, so ours' output measures each.

# shellcheck source=/dev/null
source "bench/pairs/numbers-csv.sh"

# The sum of a ZIP's entry sizes before compression.
unzipped_bytes() {
  python3 -c 'import sys, zipfile; print(sum(info.file_size for info in zipfile.ZipFile(sys.argv[1]).infolist()))' "$1"
}

run_pair() {
  numbers_inputs
  echo "== running" >&2
  for name in real dense sheets; do
    local input="$data/$name.numbers" bytes ours parser office out_bytes handled seconds pseconds oseconds rss prss orss lo_mbps lo_rss verdict
    bytes="$(wc -c < "$input" | tr -d ' ')"
    rm -rf "$data/lo-$name-xlsx"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/out-$name.xlsx")"
    parser="$(time_cmd numbers-parser "$python" bench/pairs/numbers-parser-xlsx.py "$input" "$data/out-$name-np.xlsx")"
    # Ours and the references write the same cells, so ours' output
    # measures the work for each.
    out_bytes="$(unzipped_bytes "$data/out-$name.xlsx")"
    handled=$((bytes + out_bytes))
    seconds="$(seconds_of "$ours")"; rss="$(rss_of "$ours")"
    pseconds="$(seconds_of "$parser")"; prss="$(rss_of "$parser")"
    lo_mbps="n/a"; lo_rss="n/a"
    verdict="$(echo "$seconds <= $pseconds" | bc -l)"
    if [ "$name" = real ] && command -v soffice >/dev/null; then
      office="$(time_cmd libreoffice sh -c 'soffice --headless --convert-to xlsx --outdir "$1" "$2" >/dev/null 2>&1' sh "$data/lo-$name-xlsx" "$input")"
      oseconds="$(seconds_of "$office")"; orss="$(rss_of "$office")"
      lo_mbps="$(mbps "$handled" "$oseconds")"; lo_rss="$(rss_mb "$orss")"
      verdict="$(echo "$seconds <= $pseconds && $seconds <= $oseconds" | bc -l)"
    fi
    row "numbers -> xlsx, ${name} ($(mb "$bytes") MB in + $(mb "$out_bytes") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$handled" "$seconds")" "$(mbps "$handled" "$pseconds")" "$lo_mbps" "$(pass "$verdict")"
    verdict="$(echo "$rss <= $prss" | bc -l)"
    [ "$lo_rss" != n/a ] && verdict="$(echo "$rss <= $prss && $rss <= $orss" | bc -l)"
    row "numbers -> xlsx, ${name}: peak memory (MB)" "$(rss_mb "$rss")" "$(rss_mb "$prss")" "$lo_rss" "$(pass "$verdict")"
  done
}
