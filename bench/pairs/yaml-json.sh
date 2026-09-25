#!/usr/bin/env bash
# YAML <-> JSON pair. Sourced by bench/run.sh. Two shapes per direction:
# dense (a sequence of mappings of short mixed scalars) and prose (mappings
# holding literal block scalar paragraphs), capped at 300,000 units.
# The reference is serde_yaml with serde_json.

yaml_inputs() {
  local units="$rows"
  if [ "$units" -gt 300000 ]; then
    units=300000
  fi
  echo "== generating ${units} units" >&2
  [ -f "$data/yaml-dense.yaml" ] || "$bench" yaml-json gen-yaml "$units" "$data/yaml-dense.yaml"
  [ -f "$data/yaml-prose.yaml" ] || "$bench" yaml-json gen-yaml-prose "$units" "$data/yaml-prose.yaml"
  [ -f "$data/yaml-dense.json" ] || "$bench" yaml-json gen-json "$units" "$data/yaml-dense.json"
  [ -f "$data/yaml-prose.json" ] || "$bench" yaml-json gen-json-prose "$units" "$data/yaml-prose.json"
}

run_pair() {
  yaml_inputs
  echo "== running" >&2
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/yaml-$shape.yaml"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/yaml-$shape-ours.json")"
    crates="$(time_cmd crates "$bench" yaml-json crates-yaml-json "$input" "$data/yaml-$shape-crates.json")"
    row "yaml -> json, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (serde_yaml + serde_json)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "yaml -> json, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (serde_yaml + serde_json)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
  for shape in dense prose; do
    local input bytes ours crates
    input="$data/yaml-$shape.json"
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/yaml-$shape-ours.yaml")"
    crates="$(time_cmd crates "$bench" yaml-json crates-json-yaml "$input" "$data/yaml-$shape-crates.yaml")"
    row "json -> yaml, ${shape} ($(mb "$bytes") MB): throughput (MB/s of input)" "$(mbps "$bytes" "$(seconds_of "$ours")")" "$(mbps "$bytes" "$(seconds_of "$crates")") (serde_json + serde_yaml)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates")" | bc -l)")"
    row "json -> yaml, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (serde_json + serde_yaml)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates")" | bc -l)")"
  done
}
