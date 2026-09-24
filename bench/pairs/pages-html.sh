#!/usr/bin/env bash
# Pages -> html. Sourced by bench/run.sh. Inputs and goals come from the
# pages-json pair: no other tool reads the modern Pages format, so the
# reference column holds the goals every Pages pair shares.

# shellcheck source=/dev/null
source "bench/pairs/pages-json.sh"

run_pair() {
  pages_inputs
  echo "== running" >&2
  for name in styled prose; do
    local input="$data/$name.pages" bytes ours
    bytes="$(wc -c < "$input" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$input" "$data/$name.html" --to html)"
    pages_rows pages html "$name" "$bytes" "$ours"
  done
}
