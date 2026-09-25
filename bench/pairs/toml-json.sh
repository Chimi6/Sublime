#!/usr/bin/env bash
# TOML <-> JSON pair. Sourced by bench/run.sh. Two shapes per direction:
# dense (an array of tables of short mixed scalars) and prose (header
# tables holding multi-line string paragraphs), capped at 300,000 units.
# The reference is the toml crate with serde_json.

toml_inputs() {
  local units="$rows"
  if [ "$units" -gt 300000 ]; then
    units=300000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/toml-dense.toml" ] || "$bench" toml-json gen-toml "$units" "$data/toml-dense.toml"
  [ -f "$data/toml-prose.toml" ] || "$bench" toml-json gen-toml-prose "$units" "$data/toml-prose.toml"
  [ -f "$data/toml-dense.json" ] || "$bench" toml-json gen-json "$units" "$data/toml-dense.json"
  [ -f "$data/toml-prose.json" ] || "$bench" toml-json gen-json-prose "$units" "$data/toml-prose.json"
}

run_pair() {
  toml_inputs
  echo "== running" >&2
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/toml-$shape.toml"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/toml-$shape-ours.json")"
    crates="$(time_cmd crates "$bench" toml-json crates-toml-json "$input" "$data/toml-$shape-crates.json")"
    row "toml -> json, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (toml + serde_json)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "toml -> json, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (toml + serde_json)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/toml-$shape.json"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/toml-$shape-ours.toml")"
    crates="$(time_cmd crates "$bench" toml-json crates-json-toml "$input" "$data/toml-$shape-crates.toml")"
    row "json -> toml, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (serde_json + toml)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "json -> toml, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (serde_json + toml)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
