#!/usr/bin/env bash
# TIFF <-> PNG pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images; the TIFF inputs
# are ImageMagick's LZW with the horizontal predictor. Decode reads TIFF
# and writes PNG; encode reads PNG and writes TIFF (deflate with the
# predictor on both sides). References: the image crate's TIFF decoder
# and the tiff crate's encoder, with the png crate at its default level.
# A stock image at bench/data/stock.png (or $SUBLIME_STOCK_PNG) adds
# [stock] rows.

run_pair() {
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  local stock="${SUBLIME_STOCK_PNG:-$data/stock.png}"
  local shapes="photo flat"
  [ -f "$stock" ] && shapes="photo flat stock"
  for shape in $shapes; do
    local png="$data/j$shape.png"
    [ "$shape" = stock ] && png="$stock"
    if [ ! -f "$data/f$shape.tif" ]; then
      magick "$png" -compress LZW -define tiff:predictor=2 "$data/f$shape.tif" \
        || { echo "ImageMagick is needed to generate the TIFF inputs" >&2; return 1; }
    fi
  done

  echo "== running" >&2
  for shape in $shapes; do
    local png tiff png_bytes tiff_bytes pixels ours crates tag=""
    png="$data/j$shape.png"
    [ "$shape" = stock ] && { png="$stock"; tag=" [stock]"; }
    tiff="$data/f$shape.tif"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    tiff_bytes="$(wc -c < "$tiff" | tr -d ' ')"
    pixels="$("$bench" tiff-png pixels "$tiff")"
    ours="$(time_cmd ours "$sublime" -q convert "$tiff" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" tiff-png crates-tiff-png "$tiff" "$data/out-$shape-crates.png")"
    row "tiff -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$tiff_bytes") MB on disk): throughput (MB/s of decoded pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "tiff -> png, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "tiff -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.tif")"
    crates="$(time_cmd crates "$bench" tiff-png crates-png-tiff "$png" "$data/out-$shape-crates.tif")"
    row "png -> tiff, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)$tag" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> tiff, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    rm -f "$data/out-$shape-"*
  done
}
