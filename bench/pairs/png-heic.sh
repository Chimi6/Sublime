#!/usr/bin/env bash
# PNG -> HEIC: encoding. Sourced by bench/run.sh. Reference: libheif's
# `heif-enc` (x265 at its slow preset, tuned for SSIM, as libheif runs
# it); macOS `sips` (Apple's hardware encoder) is context.
#
# Speed and memory on the jpeg-png pair's 4000 by 4000 photo and flat
# images, and on the stock 6016 by 6016 Sonoma wallpaper (decoded to PNG)
# when it exists. Two sets of photographs, each fetched once into
# bench/data: the 24 Kodak photographs (768 by 512) and 12 photographs
# from Wikimedia Commons' featured pictures, scaled to a phone's 4032
# pixels across (by `sips`; the rows are skipped without it). On each,
# every encoder codes every photograph at six qualities; every file is
# decoded by the same decoder (ours, bit-exact with libheif's planes) and
# scored against its source, and the curves compared by BD-rate (the mean
# size difference at equal quality) over PSNR of luma, SSIM, PSNR of RGB,
# and SSIMULACRA2 when `ssimulacra2` is installed. The 12 photographs are
# also timed at every `--effort`.

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
    echo "== the Kodak photographs could not be fetched: no Kodak rows" >&2
  else
    compression "Kodak (24 photographs)" "$kodak"
    local kodak_pixels=$((768 * 512 * 3 * 24))
    row "png -> heic, Kodak (24 photographs of 0.4 megapixels, quality 50): throughput (MB/s of pixels)" "$(mbps "$kodak_pixels" "$seconds_ours")" "$(mbps "$kodak_pixels" "$seconds_reference")" "$(mbps "$kodak_pixels" "$seconds_apple")" "$(pass "$(echo "$seconds_ours <= $seconds_reference" | bc -l)")"
  fi

  local photos="$data/photos"
  if fetch_photos "$photos"; then
    compression "12 photographs of 4032 pixels" "$photos"
    efforts "$photos"
  else
    echo "== the photographs could not be fetched or scaled: no photograph rows" >&2
  fi
}

# The 12 photographs: Wikimedia Commons featured pictures (each under its
# own free licence, named on its file page), checked by the first 16 hex
# digits of their SHA-256 and scaled to 4032 pixels across.
PHOTOS="
landscapes0 55815b021084ade0 9/96/01-%E0%B8%9E%E0%B8%A3%E0%B8%B0%E0%B8%97%E0%B8%B5%E0%B9%88%E0%B8%99%E0%B8%B1%E0%B9%88%E0%B8%87%E0%B8%84%E0%B8%B9%E0%B8%AB%E0%B8%B2%E0%B8%84%E0%B8%A4%E0%B8%AB%E0%B8%B2%E0%B8%AA%E0%B8%99%E0%B9%8C.jpg
landscapes1 ede85d2260f4f347 4/41/Gasoducto_junto_a_la_B-145%2C_Chile%2C_2016-02-09%2C_DD_32.JPG
landscapes2 8290c2513bd97e46 b/bd/Li_Phi_falls_at_sunrise_with_colorful_clouds_in_Don_Khon_Laos.jpg
people0 32da8862fbe9acd7 2/29/Campamento_de_ganado_de_la_tribu_Mundari%2C_Terekeka%2C_Sud%C3%A1n_del_Sur%2C_2024-01-30%2C_DD_50.jpg
people1 bbd50291d2933d40 5/58/Joseph_Kriehuber%2C_Ein_Matin%C3%A9e_bei_Liszt%2C_1846.jpg
people2 f5e09d48a365f626 1/11/Miranda_en_la_Carraca_by_Arturo_Michelena.jpg
architecture0 30bfbc3af98a870d 5/54/Kimono_Forest_at_night%2C_Arashiyama_Station%2C_Arashiyama%2C_Kyoto%2C_Japan.jpg
architecture1 8a8337eb7eee6999 f/f0/Pers%C3%A9polis%2C_Ir%C3%A1n%2C_2016-09-24%2C_DD_53.jpg
plants0 3fe5611f73a969fc c/c3/D%C3%BClmen%2C_Hausd%C3%BClmen%2C_eisbedeckter_Strauch_--_2021_--_5033-7.jpg
birds0 4176741360ff3d38 9/9d/Erfurt_-_Th%C3%BCringer_Zoopark_-_Rhea_americana_01.jpg
objects0 b609ac91d821325c d/df/Leg_Rowing_Fisherman_Inle_Lake_Myanmar.jpg
cityscapes0 0d1bbad8db466f87 7/7f/Havenwelten_-_Bremerhaven_01.jpg
"

fetch_photos() {
  local folder="$1" name sum path
  command -v sips >/dev/null || return 1
  mkdir -p "$folder"
  while read -r name sum path; do
    [ -n "$name" ] || continue
    [ -f "$folder/$name.png" ] && continue
    # Wikimedia asks for a descriptive agent and backs busy clients off.
    local tries=0
    until curl -sf -A "sublime-bench/1.0 (https://github.com/Chimi6/Sublime)" \
      -o "$folder/$name.jpg" "https://upload.wikimedia.org/wikipedia/commons/$path"; do
      tries=$((tries + 1))
      [ "$tries" -lt 4 ] || return 1
      sleep $((tries * 10))
    done
    [ "$(shasum -a 256 "$folder/$name.jpg" | cut -c1-16)" = "$sum" ] || { echo "== $name changed upstream" >&2; return 1; }
    sips -s format png --resampleWidth 4032 "$folder/$name.jpg" --out "$folder/$name.png" >/dev/null || return 1
    rm -f "$folder/$name.jpg"
    sleep 1
  done <<< "$PHOTOS"
}

# One line of scores for a coded file: PSNR of RGB, of luma, SSIM, and
# SSIMULACRA2 when installed.
scores() {
  local png="$1" heic="$2" line
  line="$("$bench" heic-png quality "$png" "$heic")"
  if command -v ssimulacra2 >/dev/null; then
    "$sublime" -q convert "$heic" "$data/out-score.png"
    line="$line $(ssimulacra2 "$png" "$data/out-score.png" | awk '{print $NF}')"
  fi
  echo "$line"
}

# Every encoder at six qualities over the set's photographs, its BD rows,
# and the quality-50 seconds of each encoder (in seconds_ours,
# seconds_reference, seconds_apple).
compression() {
  local label="$1" folder="$2"
  echo "== compression: $label, 6 qualities, 3 encoders" >&2
  local points="$data/heic-points.txt"
  : > "$points"
  seconds_ours=0 seconds_reference=0 seconds_apple=0
  for png in "$folder"/*.png; do
    local image
    image="$(basename "$png" .png)"
    for quality in 30 40 50 60 70 80; do
      local out start
      for encoder in ours x265 sips; do
        out="$data/out-set-$encoder.heic"
        rm -f "$out"
        start="$(date +%s.%N)"
        case "$encoder" in
          ours) "$sublime" -q convert "$png" "$out" --quality "$quality" ;;
          x265) heif-enc -q "$quality" "$png" -o "$out" >/dev/null ;;
          sips)
            command -v sips >/dev/null || continue
            # sips's own scale is a percent; its 35 to 93 spans the others'.
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
        echo "$encoder $image $(wc -c < "$out" | tr -d ' ') $(scores "$png" "$out")" >> "$points"
      done
    done
  done
  local against_reference against_apple="" metric index
  against_reference="$("$bench" heic-png bd "$points" ours x265)"
  command -v sips >/dev/null && against_apple="$("$bench" heic-png bd "$points" ours sips)"
  index=1
  for metric in "PSNR of luma" "SSIM" "PSNR of RGB" "SSIMULACRA2"; do
    local value apple_value="n/a"
    value="$(echo "$against_reference" | awk -v i="$index" '{print $i}')"
    [ -n "$value" ] || break
    [ -n "$against_apple" ] && apple_value="ours $(echo "$against_apple" | awk -v i="$index" '{print $i}')%"
    row "png -> heic, $label: BD-rate at equal ${metric} (negative: ours smaller)" "${value}%" "0% (itself)" "$apple_value" "$(pass "$(echo "${value#+} <= 0" | bc -l)")"
    index=$((index + 1))
  done
}

# The set at quality 50 at each effort, one run a photograph, against one
# run of heif-enc each.
efforts() {
  local folder="$1" pixels=0 png reference=0 took start effort
  echo "== effort: $folder at quality 50" >&2
  for png in "$folder"/*.png; do
    pixels=$((pixels + $("$bench" jpeg-png pixels "$png")))
    start="$(date +%s.%N)"
    heif-enc -q 50 "$png" -o "$data/out-effort-x265.heic" >/dev/null
    reference="$(echo "$reference + $(date +%s.%N) - $start" | bc -l)"
  done
  for effort in fast balanced max; do
    local seconds=0
    for png in "$folder"/*.png; do
      start="$(date +%s.%N)"
      "$sublime" -q convert "$png" "$data/out-effort-ours.heic" --effort "$effort"
      seconds="$(echo "$seconds + $(date +%s.%N) - $start" | bc -l)"
    done
    # Fast is the pass line; balanced and max buy size with time.
    local result="context"
    [ "$effort" = fast ] && result="$(pass "$(echo "$seconds <= $reference" | bc -l)")"
    row "png -> heic, 12 photographs of 4032 pixels (quality 50, --effort $effort): throughput (MB/s of pixels)" "$(mbps "$pixels" "$seconds")" "$(mbps "$pixels" "$reference")" "n/a" "$result"
  done
}
