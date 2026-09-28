#!/usr/bin/env bash
# ICO <-> PNG pair. Sourced by bench/run.sh. Decode reads a Pillow icon
# (the seven standard sizes up to 256, PNG entries) and writes the
# largest as PNG; encode makes an icon of the standard sizes from a PNG:
# a 256 by 256 image (a favicon source) and the jpeg-png pair's 4000 by
# 4000 photo and flat images (a favicon from a large picture).
# References: the image crate (its ICO decoder; for icons, `thumbnail`
# at each size and its multi-image ICO encoder) with the png crate.

run_pair() {
  local side=4000
  echo "== generating" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  if [ ! -f "$data/icon256.png" ] || [ ! -f "$data/icon.ico" ]; then
    python3 -c "
from PIL import Image
image = Image.open('$data/jphoto.png').convert('RGBA').resize((256, 256), Image.Resampling.LANCZOS)
image.save('$data/icon256.png')
image.save('$data/icon.ico')
" || { echo "Pillow is needed to generate the icon inputs" >&2; return 1; }
  fi
  echo "== running" >&2
  local ours crates bytes
  bytes="$(wc -c < "$data/icon.ico" | tr -d ' ')"
  ours="$(time_cmd ours "$sublime" -q convert "$data/icon.ico" "$data/out-icon-ours.png")"
  crates="$(time_cmd crates "$bench" ico-png crates-ico-png "$data/icon.ico" "$data/out-icon-crates.png")"
  row "ico -> png, 256 icon ($(mb "$bytes") MB): wall time (ms)" "$(awk "BEGIN{printf \"%.1f\", $(seconds_of "$ours") * 1000}")" "$(awk "BEGIN{printf \"%.1f\", $(seconds_of "$crates") * 1000}") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
  row "ico -> png, 256 icon: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  for shape in icon256 jphoto jflat; do
    local png="$data/$shape.png" png_bytes
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.ico")"
    crates="$(time_cmd crates "$bench" ico-png crates-png-ico "$png" "$data/out-$shape-crates.ico")"
    row "png -> ico, $shape ($(mb "$png_bytes") MB in): wall time (ms)" "$(awk "BEGIN{printf \"%.1f\", $(seconds_of "$ours") * 1000}")" "$(awk "BEGIN{printf \"%.1f\", $(seconds_of "$crates") * 1000}") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> ico, $shape: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "png -> ico, $shape: output size (KB) [extra]" "$(awk "BEGIN{printf \"%.1f\", $(wc -c < "$data/out-$shape-ours.ico") / 1024}")" "$(awk "BEGIN{printf \"%.1f\", $(wc -c < "$data/out-$shape-crates.ico") / 1024}") (image)" "n/a"
  done
  rm -f "$data/out-"*
}
