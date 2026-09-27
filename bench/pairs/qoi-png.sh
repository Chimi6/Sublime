#!/usr/bin/env bash
# QOI <-> PNG pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images; the QOI inputs
# are Pillow's (the qoi.h algorithm). Decode reads QOI and writes PNG;
# encode reads PNG and writes QOI. References: the qoi crate (what the
# image crate uses) with the png crate at its default level. A stock
# image at bench/data/stock.png (or $SUBLIME_STOCK_PNG) adds [stock]
# rows when present.

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
    if [ ! -f "$data/q$shape.qoi" ]; then
      python3 -c "
from PIL import Image
Image.MAX_IMAGE_PIXELS = None
image = Image.open('$png'); image.load()
image.save('$data/q$shape.qoi', 'QOI')
" || { echo "Pillow with QOI is needed to generate the QOI inputs" >&2; return 1; }
    fi
  done

  echo "== running" >&2
  for shape in $shapes; do
    local png qoi png_bytes qoi_bytes pixels ours crates tag=""
    png="$data/j$shape.png"
    [ "$shape" = stock ] && { png="$stock"; tag=" [stock]"; }
    qoi="$data/q$shape.qoi"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    qoi_bytes="$(wc -c < "$qoi" | tr -d ' ')"
    pixels="$("$bench" qoi-png pixels "$qoi")"
    ours="$(time_cmd ours "$sublime" -q convert "$qoi" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" qoi-png crates-qoi-png "$qoi" "$data/out-$shape-crates.png")"
    row "qoi -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$qoi_bytes") MB on disk): throughput (MB/s of decoded pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (qoi + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "qoi -> png, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (qoi + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "qoi -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.qoi")"
    crates="$(time_cmd crates "$bench" qoi-png crates-png-qoi "$png" "$data/out-$shape-crates.qoi")"
    row "png -> qoi, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)$tag" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + qoi)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> qoi, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + qoi)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    rm -f "$data/out-$shape-"*
  done
}
