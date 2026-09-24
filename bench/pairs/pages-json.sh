#!/usr/bin/env bash
# Pages <-> pages-json. Sourced by bench/run.sh. Inputs are fixtures scaled
# by the harness generator: `styled` repeats text-styles.pages (short
# paragraphs, many character style runs) and `prose` repeats
# paragraphs.pages (long paragraphs, few runs). There is no peer
# implementation of the modern Pages format; the reference is the floor
# of the pair (unzip and Snappy only, `bench pages-json floor`), and the
# reverse direction is held to the forward one on the same bytes.

pages_inputs() {
  local styled_units="$rows" prose_units="$rows"
  [ "$styled_units" -gt 5000 ] && styled_units=5000
  [ "$prose_units" -gt 2000 ] && prose_units=2000
  echo "== generating styled x${styled_units}, prose x${prose_units}" >&2
  [ -f "$data/styled.pages" ] || "$bench" pages-json gen tests/fixtures/pages/text-styles.pages "$styled_units" "$data/styled.pages"
  [ -f "$data/prose.pages" ] || "$bench" pages-json gen tests/fixtures/pages/paragraphs.pages "$prose_units" "$data/prose.pages"
}

run_pair() {
  pages_inputs
  echo "== running" >&2
  for name in styled prose; do
    local input="$data/$name.pages" pages_bytes json_bytes forward backward floor
    pages_bytes="$(wc -c < "$input" | tr -d ' ')"
    forward="$(time_cmd to-json "$sublime" -q convert "$input" "$data/$name.json" --to pages-json)"
    backward="$(time_cmd back "$sublime" -q convert "$data/$name.json" "$data/$name-back.pages" --from pages-json --to pages)"
    floor="$(time_cmd floor "$bench" pages-json floor "$input" "$data/$name.raw")"
    json_bytes="$(wc -c < "$data/$name.json" | tr -d ' ')"
    local raw_bytes fs bs fl
    raw_bytes="$(wc -c < "$data/$name.raw" | tr -d ' ')"
    fs="$(seconds_of "$forward")"; bs="$(seconds_of "$backward")"; fl="$(seconds_of "$floor")"
    # No peer reads the modern Pages format. The floor (unzip and Snappy
    # only) bounds what any reader could gain; ours must stay within four
    # times it while decoding every object and writing the JSON.
    row "pages -> pages-json, ${name}: MB/s of the package (decompressed streams)" "$(mbps "$pages_bytes" "$fs") ($(mbps "$raw_bytes" "$fs"))" "$(mbps "$pages_bytes" "$fl") ($(mbps "$raw_bytes" "$fl")) floor" "$(pass "$(echo "$fs <= 4 * $fl" | bc -l)")"
    row "pages-json -> pages, ${name}: MB/s of the JSON" "$(mbps "$json_bytes" "$bs")" "$(mbps "$json_bytes" "$fs") (pages -> pages-json, same bytes)" "$(pass "$(echo "$bs <= $fs" | bc -l)")"
    row "Peak RSS pages -> pages-json, ${name} (MB)" "$(rss_mb "$(rss_of "$forward")")" "$(rss_mb "$(rss_of "$floor")") floor; <= 64" "$(pass "$(echo "$(rss_of "$forward") <= 65536" | bc -l)")"
    row "Peak RSS pages-json -> pages, ${name} (MB)" "$(rss_mb "$(rss_of "$backward")")" "<= 64" "$(pass "$(echo "$(rss_of "$backward") <= 65536" | bc -l)")"
  done
}
