#!/usr/bin/env bash
# TGA <-> PNG pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images; the TGA inputs
# are Pillow's defaults (RLE, bottom-up). Decode reads TGA and writes
# PNG; encode reads PNG and writes TGA. References: the image crate's TGA
# codec (RLE) with the png crate at its default level. A stock image at
# bench/data/stock.png (or $SUBLIME_STOCK_PNG) adds [stock] rows.

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
    if [ ! -f "$data/t$shape.tga" ]; then
      python3 -c "
from PIL import Image
Image.MAX_IMAGE_PIXELS = None
image = Image.open('$png'); image.load()
image.save('$data/t$shape.tga', 'TGA', rle=True)
" || { echo "Pillow is needed to generate the TGA inputs" >&2; return 1; }
    fi
  done

  echo "== running" >&2
  for shape in $shapes; do
    local png tga png_bytes tga_bytes pixels ours crates tag=""
    png="$data/j$shape.png"
    [ "$shape" = stock ] && { png="$stock"; tag=" [stock]"; }
    tga="$data/t$shape.tga"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    tga_bytes="$(wc -c < "$tga" | tr -d ' ')"
    pixels="$("$bench" tga-png pixels "$tga")"
    ours="$(time_cmd ours "$sublime" -q convert "$tga" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" tga-png crates-tga-png "$tga" "$data/out-$shape-crates.png")"
    row "tga -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$tga_bytes") MB on disk): throughput (MB/s of decoded pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "tga -> png, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "tga -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.tga")"
    crates="$(time_cmd crates "$bench" tga-png crates-png-tga "$png" "$data/out-$shape-crates.tga")"
    row "png -> tga, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)$tag" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> tga, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    rm -f "$data/out-$shape-"*
  done
}
