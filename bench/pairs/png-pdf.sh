#!/usr/bin/env bash
# PNG <-> PDF pair. Sourced by bench/run.sh. The PNG shapes are the
# jpeg-png pair's 4000 by 4000 RGB photo and flat images; the PDF inputs
# are ImageMagick's (-compress Zip), one image per page. Decode reads the
# PDF's image and writes PNG; encode reads PNG and writes a one-page PDF.
# References: printpdf set lossless (Flate, no resize or re-encode) and
# lopdf's image lookup with the png crate at its default level. A stock
# image at bench/data/stock.png (or $SUBLIME_STOCK_PNG) adds [stock] rows.

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
    if [ ! -f "$data/d$shape.pdf" ]; then
      magick "$png" -compress Zip "$data/d$shape.pdf" \
        || { echo "ImageMagick is needed to generate the PDF inputs" >&2; return 1; }
    fi
  done

  echo "== running" >&2
  for shape in $shapes; do
    local png pdf png_bytes pdf_bytes pixels ours crates tag=""
    png="$data/j$shape.png"
    [ "$shape" = stock ] && { png="$stock"; tag=" [stock]"; }
    pdf="$data/d$shape.pdf"
    png_bytes="$(wc -c < "$png" | tr -d ' ')"
    pdf_bytes="$(wc -c < "$pdf" | tr -d ' ')"
    pixels="$("$bench" png-pdf pixels "$png")"
    ours="$(time_cmd ours "$sublime" -q convert "$pdf" "$data/out-$shape-ours.png")"
    crates="$(time_cmd crates "$bench" png-pdf crates-pdf-png "$pdf" "$data/out-$shape-crates.png")"
    row "pdf -> png, ${shape} ($(mb "$pixels") MB of pixels, $(mb "$pdf_bytes") MB on disk): throughput (MB/s of decoded pixels)$tag" "$(mbps "$pixels" "$(seconds_of "$ours")")" "$(mbps "$pixels" "$(seconds_of "$crates")") (lopdf + png)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "pdf -> png, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (lopdf + png)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "pdf -> png, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.png" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.png" | tr -d ' ')") (png)" "n/a"
    local handled=$((png_bytes + pixels))
    ours="$(time_cmd ours "$sublime" -q convert "$png" "$data/out-$shape-ours.pdf")"
    crates="$(time_cmd crates "$bench" png-pdf crates-png-pdf "$png" "$data/out-$shape-crates.pdf")"
    row "png -> pdf, ${shape} ($(mb "$png_bytes") MB in + $(mb "$pixels") MB of pixels): throughput (MB/s of input plus pixels)$tag" "$(mbps "$handled" "$(seconds_of "$ours")")" "$(mbps "$handled" "$(seconds_of "$crates")") (png + printpdf)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "png -> pdf, ${shape}: peak memory (MB)$tag" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (png + printpdf)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
    row "png -> pdf, ${shape}: output size (MB) [extra]" "$(mb "$(wc -c < "$data/out-$shape-ours.pdf" | tr -d ' ')")" "$(mb "$(wc -c < "$data/out-$shape-crates.pdf" | tr -d ' ')") (printpdf)" "n/a"
    rm -f "$data/out-$shape-"*
  done
}
