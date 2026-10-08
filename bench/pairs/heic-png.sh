#!/usr/bin/env bash
# HEIC -> PNG and HEIC -> JPEG. Sourced by bench/run.sh. The inputs are the
# jpeg-png pair's 4000 by 4000 photo and flat images encoded to HEIC by
# libheif with x265 (one picture: quality 50 and 90, flat at 75, and the
# photo at 10 bits from a 16-bit PNG) and by macOS `sips` (Apple's encoder:
# a grid of 512 by 512 tiles). Reference: libheif's `heif-dec` (libde265),
# the conventional tool; `sips` (Apple's hardware decoder) is context.
# When /System/Library/Desktop Pictures/Sonoma.heic exists (6016 by 6016,
# 36 tiles of 1024), its rows run too, marked [stock].

table_header="| Target | Ours | Reference (heif-dec) | Context (sips) | Result |"
table_sep="|---|---|---|---|---|"

run_pair() {
  command -v heif-enc >/dev/null && command -v heif-dec >/dev/null || {
    echo "libheif's heif-enc and heif-dec are needed (brew install libheif)" >&2
    return 1
  }
  local side=4000
  echo "== generating ${side}x${side}" >&2
  [ -f "$data/jphoto.png" ] || "$bench" jpeg-png gen-photo "$side" "$data/jphoto.png"
  [ -f "$data/jflat.png" ] || "$bench" jpeg-png gen-flat "$side" "$data/jflat.png"
  # 16 bits: the photo's samples above, a slow ramp below.
  if [ ! -f "$data/hphoto16.png" ]; then
  "$sublime" -q convert "$data/jphoto.png" "$data/hphoto16.pam"
  python3 - "$data/hphoto16.pam" "$data/hphoto16.png" <<'EOF'
import sys, zlib, struct
pam = open(sys.argv[1], 'rb').read()
head, body = pam.split(b'ENDHDR\n', 1)
fields = dict(line.split(b' ', 1) for line in head.split(b'\n')[1:] if b' ' in line)
w, h = int(fields[b'WIDTH']), int(fields[b'HEIGHT'])
raw = bytearray()
for y in range(h):
    raw.append(0)
    row = body[y * w * 3:(y + 1) * w * 3]
    low = bytes(((x // 3 + y) * 5) & 255 for x in range(w * 3))
    out = bytearray(w * 6); out[0::2] = row; out[1::2] = low
    raw += out
chunk = lambda t, d: struct.pack('>I', len(d)) + t + d + struct.pack('>I', zlib.crc32(t + d) & 0xffffffff)
open(sys.argv[2], 'wb').write(b'\x89PNG\r\n\x1a\n' + chunk(b'IHDR', struct.pack('>IIBBBBB', w, h, 16, 2, 0, 0, 0)) + chunk(b'IDAT', zlib.compress(bytes(raw), 6)) + chunk(b'IEND', b''))
EOF
  rm -f "$data/hphoto16.pam"
  fi
  [ -f "$data/hphoto-x265-q50.heic" ] || heif-enc -q 50 "$data/jphoto.png" -o "$data/hphoto-x265-q50.heic" >/dev/null
  [ -f "$data/hphoto-x265-q90.heic" ] || heif-enc -q 90 "$data/jphoto.png" -o "$data/hphoto-x265-q90.heic" >/dev/null
  [ -f "$data/hflat-x265-q75.heic" ] || heif-enc -q 75 "$data/jflat.png" -o "$data/hflat-x265-q75.heic" >/dev/null
  [ -f "$data/hphoto-x265-10bit.heic" ] || heif-enc -b 10 -q 75 "$data/hphoto16.png" -o "$data/hphoto-x265-10bit.heic" >/dev/null
  if command -v sips >/dev/null; then
    [ -f "$data/hphoto-apple.heic" ] || sips -s format heic "$data/jphoto.png" --out "$data/hphoto-apple.heic" >/dev/null
  fi

  echo "== running" >&2
  local inputs="hphoto-x265-q50 hphoto-x265-q90 hphoto-x265-10bit hflat-x265-q75"
  [ -f "$data/hphoto-apple.heic" ] && inputs="$inputs hphoto-apple"
  local stock="/System/Library/Desktop Pictures/Sonoma.heic"
  for name in $inputs stock; do
    local heic="$data/$name.heic" tag=""
    if [ "$name" = stock ]; then
      [ -f "$stock" ] || continue
      cp "$stock" "$data/stock.heic"
      heic="$data/stock.heic"; tag=" [stock]"
    fi
    local pixels heic_bytes
    pixels="$(heif-info "$heic" | awk '/^image:/ {split($2, s, "x"); print s[1] * s[2] * 3; exit}')"
    heic_bytes="$(wc -c < "$heic" | tr -d ' ')"
    for ext in png jpg; do
      local ours libheif apple apple_cell="n/a"
      ours="$(time_cmd ours "$sublime" -q convert "$heic" "$data/out-$name-ours.$ext")"
      # heif-dec reports on stdout for files of several images.
      libheif="$(time_cmd libheif bash -c 'heif-dec "$0" "$1" >/dev/null' "$heic" "$data/out-$name-libheif.$ext")"
      if command -v sips >/dev/null; then
        local format=png; [ "$ext" = jpg ] && format=jpeg
        # sips names its files on stdout; the timing line must stay clean.
        apple="$(time_cmd sips bash -c 'sips -s format "$0" "$1" --out "$2" >/dev/null' "$format" "$heic" "$data/out-$name-sips.$ext")"
        apple_cell="$(mbps "$pixels" "$(seconds_of "$apple")")"
      fi
      local target="heic -> ${ext/jpg/jpeg}, ${name} ($(mb "$pixels") MB of pixels, $(mb "$heic_bytes") MB on disk)"
      row "$target: throughput (MB/s of decoded pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$libheif")")" "$apple_cell" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$libheif")" | bc -l)")"
      local apple_rss="n/a"
      [ -n "${apple:-}" ] && apple_rss="$(rss_mb "$(rss_of "$apple")")"
      row "heic -> ${ext/jpg/jpeg}, ${name}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$libheif")")" "$apple_rss" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$libheif")" | bc -l)")"
    done
  done
  rm -f "$data/stock.heic"
}
