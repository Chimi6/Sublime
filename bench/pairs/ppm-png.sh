#!/usr/bin/env bash
# Netpbm <-> PNG pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images, as PPM (P6);
# a stock image at bench/data/stock.png (or $SUBLIME_STOCK_PNG, RGBA)
# adds [stock] rows as PAM (P7). The Netpbm inputs are ours (the format
# has one way to write a raw 8-bit file). References: the image crate's
# PNM codec with the png crate at its default level.

run_pair() {
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  local stock="${SUBLIME_STOCK_PNG:-$data/stock.png}"
  local shapes="photo flat"
  [ -f "$stock" ] && shapes="photo flat stock"
  echo "== running" >&2
  for shape in $shapes; do
    local png="$data/j$shape.png" kind=ppm tag=""
    [ "$shape" = stock ] && { png="$stock"; kind=pam; tag=" [stock]"; }
    local netpbm="$data/n$shape.$kind"
    [ -f "$netpbm" ] || "$sublime" -q convert "$png" "$netpbm"
    local png_bytes pixels ours crates
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    pixels="$(wc -c < "$netpbm" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$netpbm" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" ppm-png crates-pnm-png "$netpbm" "$data/out-$shape-crates.png")"
    row "$kind -> png, ${shape} ($(mb "$pixels") MB in): throughput (MB/s of input)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "$kind -> png, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "$kind -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.$kind")"
    crates="$(time_cmd crates "$bench" ppm-png crates-png-$kind "$png" "$data/out-$shape-crates.$kind")"
    row "png -> $kind, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB out): throughput (MB/s of input plus output)$tag" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> $kind, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    rm -f "$data/out-$shape-"*
  done
}
