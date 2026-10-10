#!/usr/bin/env bash
# PNG -> HEIC: encoding. Sourced by bench/run.sh. Reference: libheif's
# `heif-enc` (x265 at its slow preset, tuned for SSIM, as libheif runs
# it); macOS `sips` (Apple's hardware encoder) is context.
#
# Speed and memory on the jpeg-png pair's 4000 by 4000 photo and flat
# images, and on the stock 6016 by 6016 Sonoma wallpaper (decoded to PNG)
# when it exists. Compression on the 24 Kodak photographs (fetched once
# into bench/data/kodak): each encoder at six qualities, every file
# decoded by the same decoder (ours, bit-exact with libheif's planes) and
# scored against its source, the curves compared by BD-rate (the mean
# size difference at equal quality) over PSNR of luma, SSIM, and PSNR of
# RGB.

table_header="| Target | Ours | Reference (heif-enc) | Context (sips) | Result |"
table_sep="|---|---|---|---|---|"

run_pair() {
  command -v heif-enc >/dev/null || {
    echo "libheif's heif-enc is needed (brew install libheif)" >&2
    return 1
  }
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  local inputs="jphoto jflat"
  local stock="/System/Library/Desktop Pictures/Sonoma.heic"
  if [ -f "$stock" ]; then
    [ -f "$data/hstock.png" ] || "$sublime" -q convert "$stock" "$data/hstock.png"
    inputs="$inputs hstock"
  fi
  echo "== speed and memory" >&2
  for name in $inputs; do
    local png="$data/$name.png" tag=""
    [ "$name" = hstock ] && tag=" [stock]"
    local pixels
    pixels="$("$bench" jpeg-png pixels "$png")"
    local ours reference apple apple_speed="n/a" apple_rss="n/a"
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$name-ours.heic")"
    reference="$(time_cmd heif-enc bash -c 'heif-enc -q 50 "$0" -o "$1" >/dev/null' "$png" "$data/out-$name-x265.heic")"
    if command -v sips >/dev/null; then
      apple="$(time_cmd sips bash -c 'sips -s format heic "$0" --out "$1" >/dev/null' "$png" "$data/out-$name-sips.heic")"
      apple_speed="$(mbps "$pixels" "$(seconds_of "$apple")")"
      apple_rss="$(rss_mb "$(rss_of "$apple")")"
    fi
    local target="png -> heic, ${name} ($(mb "$pixels") MB of pixels, quality 50)"
    row "$target: throughput (MB/s of pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$reference")")" "$apple_speed" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$reference")" | bc -l)")"
    row "png -> heic, ${name}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$reference")")" "$apple_rss" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$reference")" | bc -l)")"
  done

  local kodak="$data/kodak"
  mkdir -p "$kodak"
  for index in $(seq -w 1 24); do
    [ -f "$kodak/kodim$index.png" ] || curl -sf -o "$kodak/kodim$index.png" \
      "https://r0k.us/graphics/kodak/kodak/kodim$index.png" || rm -f "$kodak/kodim$index.png"
  done
  if [ "$(ls "$kodak"/kodim*.png 2>/dev/null | wc -l)" -lt 24 ]; then
    echo "== the Kodak photographs could not be fetched: no compression rows" >&2
    return 0
  fi
  echo "== compression (24 photographs, 6 qualities, 3 encoders)" >&2
  local points="$data/heic-points.txt"
  : > "$points"
  local seconds_ours=0 seconds_reference=0 seconds_apple=0
  for png in "$kodak"/kodim*.png; do
    local image
    image="$(basename "$png" .png)"
    for quality in 30 40 50 60 70 80; do
      local out start
      for encoder in ours x265 sips; do
        out="$data/out-kodak-$encoder.heic"
        rm -f "$out"
        start="$(date +%s.%N)"
        case "$encoder" in
          ours) "$sublime" -q convert "$png" "$out" --quality "$quality" ;;
          x265) heif-enc -q "$quality" "$png" -o "$out" >/dev/null ;;
          sips)
            command -v sips >/dev/null || continue
            # sips's own scale is a percent; its 30 to 95 spans the others'.
            sips -s format heic -s formatOptions "$((quality + quality / 6))" "$png" --out "$out" >/dev/null
            ;;
        esac
        local took
        took="$(echo "$(date +%s.%N) - $start" | bc -l)"
        if [ "$quality" = 50 ]; then
          case "$encoder" in
            ours) seconds_ours="$(echo "$seconds_ours + $took" | bc -l)" ;;
            x265) seconds_reference="$(echo "$seconds_reference + $took" | bc -l)" ;;
            sips) seconds_apple="$(echo "$seconds_apple + $took" | bc -l)" ;;
          esac
        fi
        echo "$encoder $image $(wc -c < "$out" | tr -d ' ') $("$bench" heic-png quality "$png" "$out")" >> "$points"
      done
    done
  done
  local against_reference against_apple="" metric index
  against_reference="$("$bench" heic-png bd "$points" ours x265)"
  command -v sips >/dev/null && against_apple="$("$bench" heic-png bd "$points" ours sips)"
  index=1
  for metric in "PSNR of luma" "SSIM" "PSNR of RGB"; do
    local value apple_value="n/a"
    value="$(echo "$against_reference" | awk -v i="$index" '{print $i}')"
    [ -n "$against_apple" ] && apple_value="ours $(echo "$against_apple" | awk -v i="$index" '{print $i}')%"
    row "png -> heic, Kodak (24 photographs): BD-rate at equal ${metric} (negative: ours smaller)" "${value}%" "0% (itself)" "$apple_value" "$(pass "$(echo "$value <= 0" | bc -l)")"
    index=$((index + 1))
  done
  local kodak_pixels=$((768 * 512 * 3 * 24))
  row "png -> heic, Kodak (24 photographs of 0.4 megapixels, quality 50): throughput (MB/s of pixels)" "$(mbps "$kodak_pixels" "$seconds_ours")" "$(mbps "$kodak_pixels" "$seconds_reference")" "$(mbps "$kodak_pixels" "$seconds_apple")" "$(pass "$(echo "$seconds_ours <= $seconds_reference" | bc -l)")"
}
