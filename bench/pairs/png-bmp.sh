#!/usr/bin/env bash
# PNG <-> BMP pair. Sourced by bench/run.sh. Two shapes, each a 4000 by
# 4000 RGBA image written by the png crate: photo (a gradient with noise)
# and flat (solid blocks). PNG is compressed, so the reader's throughput
# counts the decoded pixel bytes and the writer's counts input plus
# pixel bytes, as the README says for compressed formats. References:
# the png crate at its default compression level (the same deflate class
# as ours) with the image crate's BMP codec; the png crate's fast level
# (fdeflate, a third larger output) is an extra row.

run_pair() {
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/photo.png" ] || "$bench" png-bmp gen-photo "$side" "$data/photo.png"
  [ -f "$data/flat.png" ] || "$bench" png-bmp gen-flat "$side" "$data/flat.png"
  [ -f "$data/photo.bmp" ] || "$sublime" -q convert "$data/photo.png" "$data/photo.bmp"
  [ -f "$data/flat.bmp" ] || "$sublime" -q convert "$data/flat.png" "$data/flat.bmp"

  echo "== running" >&2
  for shape in photo flat; do
    local png bmp png_bytes bmp_bytes pixels ours crates
    png="$data/$shape.png"; bmp="$data/$shape.bmp"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    bmp_bytes="$(wc -c < "$bmp" | tr -d ' ')"
    pixels="$("$bench" png-bmp pixels "$png")"
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.bmp")"
    crates="$(time_cmd crates "$bench" png-bmp crates-png-bmp "$png" "$data/out-$shape-crates.bmp")"
    row "png -> bmp, ${shape} ($(mb "$pixels") MB of pixels): throughput (MB/s of decoded pixels)" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (png + image)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> bmp, ${shape} ($(mb "$png_bytes") MB on disk): throughput (MB/s of file bytes) [extra]" "$(mbps "$png_bytes" "$(seconds_of "$ours")")" "$(mbps "$png_bytes" "$(seconds_of "$crates")") (png + image)" "n/a"
    row "png -> bmp, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + image)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    local handled=$((bmp_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$bmp" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" png-bmp crates-bmp-png "$bmp" "$data/out-$shape-crates.png")"
    local fast
    fast="$(time_cmd fast "$bench" png-bmp crates-bmp-png-fast "$bmp" "$data/out-$shape-fast.png")"
    row "bmp -> png, ${shape} ($(mb "$bmp_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (image + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "bmp -> png, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (image + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "bmp -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    row "bmp -> png, ${shape}: the png crate's fast level, throughput and size [extra]" "-" "$(mbps "$handled" "$(seconds_of "$fast")") MB/s, $(mb "$(wc -c < "$data/out-$shape-fast.png" | tr -d ' ')") MB (png fast)" "n/a"
  done
}
