#!/bin/bash
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
out="$SC/p2w.txt"; : > "$out"
for f in "$ROOT"/tests/fixtures/pages/*.pages "$SC"/p2wsrc/*.pages; do
  "$HERE/p2w.sh" "$f" "$(basename "$f" .pages)" >> "$out" 2>&1
done
echo DONE >> "$out"
