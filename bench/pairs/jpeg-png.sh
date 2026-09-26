#!/usr/bin/env bash
# JPEG <-> PNG pair. Sourced by bench/run.sh. Two shapes, each a 4000 by
# 4000 RGB image: photo (gradients with independent per-channel noise)
# and flat (solid blocks), as a PNG written by the png crate and as a
# JPEG at quality 85 written by the jpeg-encoder crate. Decode reads the
# JPEG and writes PNG; encode reads the PNG and writes JPEG at quality
# 85. References: the image crate (zune-jpeg to decode; its own baseline
# encoder, which subsamples 4:2:2) with the png crate, and the
# jpeg-encoder crate (a mozjpeg port, 4:2:0 below 90, like ours) as the
# tighter encode peer. Throughput counts decoded pixel bytes for the
# reader and input plus pixel bytes for the writer; every JPEG written is
# also scored by PSNR against the source pixels, since a size at "the
# same quality" says nothing when the subsampling differs. A stock
# photograph at bench/data/stock.jpg (or $SUBLIME_STOCK_JPG) adds
# [stock] rows when present.

run_pair() {
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"

  echo "== running" >&2
  for shape in photo flat; do
    local png jpg jpg_bytes png_bytes pixels ours crates
    png="$data/j$shape.png"; jpg="$data/j$shape.jpg"
    jpg_bytes="$(wc -c < "$jpg" | tr -d ' ')"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    pixels="$("$bench" jpeg-png pixels "$png")"
    ours="$(time_cmd ours "$sublime" -q convert "$jpg" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" jpeg-png crates-jpeg-png "$jpg" "$data/out-$shape-crates.png")"
    row "jpeg -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$jpg_bytes") MB on disk): throughput (MB/s of decoded pixels)" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "jpeg -> png, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.jpg")"
    crates="$(time_cmd crates "$bench" jpeg-png crates-png-jpeg "$png" "$data/out-$shape-crates.jpg")"
    local encoder
    encoder="$(time_cmd encoder "$bench" jpeg-png crates-png-jpeg-encoder "$png" "$data/out-$shape-encoder.jpg")"
    row "png -> jpeg, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$encoder")") (png + jpeg-encoder); $(mbps "$handled" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$encoder")" | bc -l)")"
    row "png -> jpeg, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$encoder")") (png + jpeg-encoder); $(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$encoder")" | bc -l)")"
    local ours_size encoder_size crates_size ours_psnr encoder_psnr crates_psnr
    ours_size="$(wc -c < "$data/out-$shape-ours.jpg" | tr -d ' ')"; encoder_size="$(wc -c < "$data/out-$shape-encoder.jpg" | tr -d ' ')"; crates_size="$(wc -c < "$data/out-$shape-crates.jpg" | tr -d ' ')"
    ours_psnr="$("$bench" jpeg-png psnr "$png" "$data/out-$shape-ours.jpg")"; encoder_psnr="$("$bench" jpeg-png psnr "$png" "$data/out-$shape-encoder.jpg")"; crates_psnr="$("$bench" jpeg-png psnr "$png" "$data/out-$shape-crates.jpg")"
    row "png -> jpeg, ${shape}: output size (MB) at quality 85 [extra]" "$(mb "$ours_size")" "$(mb "$encoder_size") (jpeg-encoder); $(mb "$crates_size") (image)" "n/a"
    row "png -> jpeg, ${shape}: PSNR against the source (dB) at quality 85 [extra]" "$ours_psnr" "$encoder_psnr (jpeg-encoder); $crates_psnr (image)" "n/a"
  done

  local stock="${SUBLIME_STOCK_JPG:-$data/stock.jpg}"
  if [ -f "$stock" ]; then
    echo "== stock image $stock" >&2
    local png jpg jpg_bytes png_bytes pixels ours crates encoder
    jpg="$stock"; png="$data/stock-from-jpg.png"
    [ -f "$png" ] || "$sublime" -q convert "$jpg" "$png"
    jpg_bytes="$(wc -c < "$jpg" | tr -d ' ')"; png_bytes="$(wc -c < "$png" | tr -d ' ')"
    pixels="$("$bench" jpeg-png pixels "$png")"
    ours="$(time_cmd ours "$sublime" -q convert "$jpg" "$data/out-stock-ours.png")"
    crates="$(time_cmd crates "$bench" jpeg-png crates-jpeg-png "$jpg" "$data/out-stock-crates.png")"
    row "jpeg -> png, stock ($(mb "$pixels") MB of pixels, $(mb "$jpg_bytes") MB on disk): throughput (MB/s of decoded pixels) [stock]" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "jpeg -> png, stock: peak memory (MB) [stock]" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-stock-ours.jpg")"
    encoder="$(time_cmd encoder "$bench" jpeg-png crates-png-jpeg-encoder "$png" "$data/out-stock-encoder.jpg")"
    row "png -> jpeg, stock ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels) [stock]" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$encoder")") (png + jpeg-encoder)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$encoder")" | bc -l)")"
    row "png -> jpeg, stock: peak memory (MB) [stock]" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$encoder")") (png + jpeg-encoder)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$encoder")" | bc -l)")"
    row "png -> jpeg, stock: output size (MB) and PSNR (dB) at quality 85 [stock]" "$(mb "$(wc -c < "$data/out-stock-ours.jpg" | tr -d ' ')") MB, $("$bench" jpeg-png psnr "$png" "$data/out-stock-ours.jpg") dB" "$(mb "$(wc -c < "$data/out-stock-encoder.jpg" | tr -d ' ')") MB, $("$bench" jpeg-png psnr "$png" "$data/out-stock-encoder.jpg") dB (jpeg-encoder)" "n/a"
    rm -f "$data/out-stock-"*
  fi
}
