#!/usr/bin/env bash
# ODS <-> CSV pair. Sourced by bench/run.sh. Inputs:
#   generated: the csv-json generator's records (capped at 1,000,000 rows)
#              as a spreadsheet LibreOffice wrote from that CSV;
#   real:      veh9901.ods, the Department for Transport's vehicle
#              licensing table (Open Government Licence), fetched by its
#              published URL: 15 MB holding 744 MB of XML, read for its
#              first sheet.
# References: calamine 0.36 with the csv crate (read) and LibreOffice (read
# and write), the reference ODS implementation. spreadsheet-ods, the Rust
# ODS writer, cannot be built (every release needs the yanked zip 6), so
# LibreOffice is the writing reference. Throughput counts the inflated
# parts (README, compressed-input and compressed-output paths).

ods_real_url="https://assets.publishing.service.gov.uk/media/6aad56203f8e4e96a6a66a39/veh9901.ods"
table_header="| Target | Ours | calamine | LibreOffice | Result |"
table_sep="|---|---|---|---|---|"
ods_lo_filter='csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,1'

unzipped_bytes() {
  python3 -c 'import sys, zipfile; print(sum(info.file_size for info in zipfile.ZipFile(sys.argv[1]).infolist()))' "$1"
}

ods_inputs() {
  local units="$rows"
  [ "$units" -gt 1000000 ] && units=1000000
  echo "== generating ${units} rows" >&2
  [ -f "$data/medium.csv" ] || "$bench" csv-json gen-csv "$units" "$data/medium.csv"
  if [ ! -f "$data/generated.ods" ]; then
    soffice --headless --convert-to ods --outdir "$data/ods-gen" "$data/medium.csv" >/dev/null 2>&1
    mv "$data/ods-gen/medium.ods" "$data/generated.ods"
  fi
  [ -f "$data/real.ods" ] || curl -sSfL -o "$data/real.ods" "${ODS_REAL_URL:-$ods_real_url}"
}

# lo_csv <input> <outdir>: LibreOffice's CSV of the first sheet; soffice
# reports on standard output, and the timing line must be the only output.
lo_time() {
  time_cmd libreoffice sh -c 'soffice --headless --convert-to "$1" --outdir "$2" "$3" >/dev/null 2>&1' sh "$1" "$2" "$3"
}

run_pair() {
  ods_inputs
  echo "== running" >&2
  local name input inflated ours crates office s c o rs rc ro
  for name in generated real; do
    input="$data/$name.ods"
    inflated="$(unzipped_bytes "$input")"
    rm -rf "$data/ods-out-$name.csv" "$data/ods-lo-$name"
    ours="$(time_cmd ours "$sublime" -q convert "$input" --sheet 1 "$data/ods-out-$name.csv")"
    crates="$(time_cmd calamine "$bench" ods-csv crates-ods-csv "$input" "$data/ods-cal-$name.csv")"
    office="$(lo_time "$ods_lo_filter" "$data/ods-lo-$name" "$input")"
    s="$(seconds_of "$ours")"; c="$(seconds_of "$crates")"; o="$(seconds_of "$office")"
    rs="$(rss_of "$ours")"; rc="$(rss_of "$crates")"; ro="$(rss_of "$office")"
    row "ods -> csv, ${name} ($(mb "$inflated") MB uncompressed): throughput (MB/s of uncompressed input)" "$(mbps "$inflated" "$s")" "$(mbps "$inflated" "$c")" "$(mbps "$inflated" "$o")" "$(pass "$(echo "$s <= $c && $s <= $o" | bc -l)")"
    row "ods -> csv, ${name}: peak memory (MB)" "$(rss_mb "$rs")" "$(rss_mb "$rc")" "$(rss_mb "$ro")" "$(pass "$(echo "$rs <= $rc && $rs <= $ro" | bc -l)")"
  done
  local csv_bytes out_inflated handled
  csv_bytes="$(wc -c < "$data/medium.csv" | tr -d ' ')"
  rm -rf "$data/ods-w.ods" "$data/ods-lo-w"
  ours="$(time_cmd ours "$sublime" -q convert "$data/medium.csv" "$data/ods-w.ods")"
  office="$(lo_time ods "$data/ods-lo-w" "$data/medium.csv")"
  out_inflated="$(unzipped_bytes "$data/ods-w.ods")"
  handled=$((csv_bytes + out_inflated))
  s="$(seconds_of "$ours")"; o="$(seconds_of "$office")"
  rs="$(rss_of "$ours")"; ro="$(rss_of "$office")"
  row "csv -> ods, generated ($(mb "$csv_bytes") MB in + $(mb "$out_inflated") MB out): throughput (MB/s of input plus uncompressed output)" "$(mbps "$handled" "$s")" "n/a" "$(mbps "$handled" "$o")" "$(pass "$(echo "$s <= $o" | bc -l)")"
  row "csv -> ods, generated: peak memory (MB)" "$(rss_mb "$rs")" "n/a" "$(rss_mb "$ro")" "$(pass "$(echo "$rs <= $ro" | bc -l)")"
}
