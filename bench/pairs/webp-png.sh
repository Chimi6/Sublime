#!/usr/bin/env bash
# WebP <-> PNG pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images; the WebP inputs
# are libwebp's (through Pillow): lossless at its default effort, and
# lossy at quality 80. Decode reads the WebP and writes PNG; encode reads
# the PNG and writes lossless WebP. References: the image crate
# (image-webp) with the png crate at its Default level. libwebp's own
# lossless size is recorded as context for the encoder's ratio. A stock
# image at bench/data/stock.webp (or $SUBLIME_STOCK_WEBP) adds [stock]
# rows when present.

run_pair() {
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  for shape in photo flat; do
    if [ ! -f "$data/w$shape-lossless.webp" ] || [ ! -f "$data/w$shape-lossy.webp" ]; then
      python3 -c "
from PIL import Image
image = Image.open('$data/j$shape.png'); image.load()
image.save('$data/w$shape-lossless.webp', lossless=True)
image.save('$data/w$shape-lossy.webp', quality=80)
" || { echo "Pillow with WebP is needed to generate the WebP inputs" >&2; return 1; }
    fi
  done

  echo "== running" >&2
  for shape in photo flat; do
    local png png_bytes pixels ours crates
    png="$data/j$shape.png"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    pixels="$("$bench" webp-png pixels "$png")"
    for kind in lossless lossy; do
      local webp webp_bytes
      webp="$data/w$shape-$kind.webp"
      webp_bytes="$(wc -c < "$webp" | tr -d ' ')"
      ours="$(time_cmd ours "$sublime" -q convert "$webp" "$data/out-$shape-$kind-ours.png")"
      crates="$(time_cmd crates "$bench" webp-png crates-webp-png "$webp" "$data/out-$shape-$kind-crates.png")"
      row "webp ($kind) -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$webp_bytes") MB on disk): throughput (MB/s of decoded pixels)" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
      row "webp ($kind) -> png, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    done
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.webp")"
    crates="$(time_cmd crates "$bench" webp-png crates-png-webp "$png" "$data/out-$shape-crates.webp")"
    row "png -> webp (lossless), ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> webp (lossless), ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "png -> webp (lossless), ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.webp" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.webp" | tr -d ' ')") (image); $(mb "$(wc -c < "$data/w$shape-lossless.webp" | tr -d ' ')") (libwebp, default effort)" "n/a"
  done

  local stock="${SUBLIME_STOCK_WEBP:-$data/stock.webp}"
  if [ -f "$stock" ]; then
    echo "== stock image $stock" >&2
    local pixels ours crates stock_bytes
    stock_bytes="$(wc -c < "$stock" | tr -d ' ')"
    pixels="$("$bench" webp-png pixels "$stock")"
    ours="$(time_cmd ours "$sublime" -q convert "$stock" "$data/out-stock-ours.png")"
    crates="$(time_cmd crates "$bench" webp-png crates-webp-png "$stock" "$data/out-stock-crates.png")"
    row "webp -> png, stock ($(mb "$pixels") MB of pixels, $(mb "$stock_bytes") MB on disk): throughput (MB/s of decoded pixels) [stock]" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "webp -> png, stock: peak memory (MB) [stock]" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    rm -f "$data/out-stock-"*
  fi
}
