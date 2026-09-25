#!/usr/bin/env bash
# XML <-> JSON pair. Sourced by bench/run.sh. Two shapes per direction:
# dense (record elements with attributes and short children) and prose
# (section elements holding paragraphs), capped at 300,000 units.
# The reference is quick-xml with serde_json doing the same mapping.

xml_inputs() {
  local units="$rows"
  if [ "$units" -gt 300000 ]; then
    units=300000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/xml-dense.xml" ] || "$bench" xml-json gen-xml "$units" "$data/xml-dense.xml"
  [ -f "$data/xml-prose.xml" ] || "$bench" xml-json gen-xml-prose "$units" "$data/xml-prose.xml"
  [ -f "$data/xml-dense.json" ] || "$bench" xml-json gen-json "$units" "$data/xml-dense.json"
  [ -f "$data/xml-prose.json" ] || "$bench" xml-json gen-json-prose "$units" "$data/xml-prose.json"
}

run_pair() {
  xml_inputs
  echo "== running" >&2
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/xml-$shape.xml"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/xml-$shape-ours.json")"
    crates="$(time_cmd crates "$bench" xml-json crates-xml-json "$input" "$data/xml-$shape-crates.json")"
    row "xml -> json, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (quick-xml + serde_json)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "xml -> json, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (quick-xml + serde_json)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/xml-$shape.json"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/xml-$shape-ours.xml")"
    crates="$(time_cmd crates "$bench" xml-json crates-json-xml "$input" "$data/xml-$shape-crates.xml")"
    row "json -> xml, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (serde_json + quick-xml)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "json -> xml, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (serde_json + quick-xml)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
