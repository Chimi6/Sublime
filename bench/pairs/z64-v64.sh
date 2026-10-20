#!/usr/bin/env bash
# N64 ROM byte orders (z64, v64, n64). Sourced by bench/run.sh. Inputs:
#   generated: 64 MB, the largest cartridge, of pseudo-random words behind
#              a big-endian header;
#   real:      a ROM of your own at $SUBLIME_N64_ROM, big-endian (not
#              distributed), adding [real] rows.
# References: n64romconvert 1.1 (the Rust converter, every direction),
# ucon64 2.2 (z64 <-> v64; it does not know little-endian n64), and
# `dd conv=swab` (byte pairs, z64 <-> v64). Every output is compared with
# ours byte for byte. Throughput is MB/s of input.

# A conversion takes tens of milliseconds, so each is timed 15 times.
bench_runs=15
table_header="| Target | Ours | n64romconvert | ucon64 | dd conv=swab | Result |"
table_sep="|---|---|---|---|---|---|"

# ucon64 writes beside its working directory under the input's name and
# prints its report; the timing line must be the only output.
ucon64_time() {
  local flag="$1" input="$2" output="$3"
  mkdir -p "$data/ucon64"
  cp "$input" "$data/ucon64/rom.${input##*.}"
  time_cmd ucon64 sh -c 'cd "$1" && rm -f "rom.$3" && ucon64 --nbak "$2" "rom.$4" >/dev/null 2>&1' sh \
    "$data/ucon64" "$flag" "${output##*.}" "${input##*.}"
  cp "$data/ucon64/rom.${output##*.}" "$output"
  rm -f "$data/ucon64/"rom.*
}

same() { if cmp -s "$1" "$2"; then echo "identical"; else echo "DIFFERS"; fi; }

run_pair() {
  local generated="$data/rom-generated.z64"
  [ -f "$generated" ] || "$bench" z64-v64 gen 67108864 "$generated"
  local real="${SUBLIME_N64_ROM:-}"
  local shapes="generated"
  [ -n "$real" ] && [ -f "$real" ] && shapes="generated real"
  for shape in $shapes; do
    local z64="$generated" tag=""
    [ "$shape" = real ] && { z64="$real"; tag=" [real]"; }
    local bytes; bytes="$(wc -c < "$z64" | tr -d ' ')"
    # The other orders, as inputs: ours, checked against the crate below.
    "$bench" z64-v64 ours z64-v64 "$z64" "$data/rom-$shape.v64"
    "$bench" z64-v64 ours z64-n64 "$z64" "$data/rom-$shape.n64"
    # Their 128 MB would otherwise still be going to disk while the first
    # row, ours, is timed.
    sync
    for direction in z64-v64 v64-z64 z64-n64 v64-n64; do
      local from="${direction%-*}" to="${direction#*-}"
      local input="$data/rom-$shape.$from"
      [ "$from" = z64 ] && input="$z64"
      local ours crate uc="" dd="" uc_cell="n/a" dd_cell="n/a" uc_mem="n/a" dd_mem="n/a"
      ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/out-ours.$to")"
      crate="$(time_cmd crate "$bench" z64-v64 crate "$direction" "$input" "$data/out-crate.$to")"
      local check="crate $(same "$data/out-ours.$to" "$data/out-crate.$to")"
      local slowest="$(seconds_of "$crate")" most="$(rss_of "$crate")"
      if [ "$to" != n64 ]; then
        uc="$(ucon64_time "--$to" "$input" "$data/out-ucon64.$to")"
        dd="$(time_cmd dd dd if="$input" bs=1048576 conv=swab status=none of="$data/out-dd.$to")"
        check="$check, ucon64 $(same "$data/out-ours.$to" "$data/out-ucon64.$to"), dd $(same "$data/out-ours.$to" "$data/out-dd.$to")"
        uc_cell="$(mbps "$bytes" "$(seconds_of "$uc")")"; dd_cell="$(mbps "$bytes" "$(seconds_of "$dd")")"
        uc_mem="$(rss_mb "$(rss_of "$uc")")"; dd_mem="$(rss_mb "$(rss_of "$dd")")"
      fi
      echo "== $direction $shape: $check" >&2
      local fastest_ok mem_ok
      fastest_ok="$(echo "$(seconds_of "$ours") <= $slowest" | bc -l)"
      mem_ok="$(echo "$(rss_of "$ours") <= $most" | bc -l)"
      if [ -n "$uc" ]; then
        fastest_ok="$(echo "$fastest_ok && $(seconds_of "$ours") <= $(seconds_of "$uc") && $(seconds_of "$ours") <= $(seconds_of "$dd")" | bc -l)"
        mem_ok="$(echo "$mem_ok && $(rss_of "$ours") <= $(rss_of "$uc") && $(rss_of "$ours") <= $(rss_of "$dd")" | bc -l)"
      fi
      row "$from -> $to, ${shape} ($(mb "$bytes") MB): throughput (MB/s)$tag" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crate")")" "$uc_cell" "$dd_cell" "$(pass "$fastest_ok")"
      row "$from -> $to, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crate")")" "$uc_mem" "$dd_mem" "$(pass "$mem_ok")"
      rm -f "$data/out-ours.$to" "$data/out-crate.$to" "$data/out-ucon64.$to" "$data/out-dd.$to"
    done
  done
}
