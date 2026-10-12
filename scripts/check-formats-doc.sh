#!/usr/bin/env bash
# Fails when DOCS/FORMATS.md or DOCS/CONVERSIONS.md does not match the registry.
set -euo pipefail
cd "$(dirname "$0")/.."
stale=0
for pair in "--markdown DOCS/FORMATS.md" "--conversions DOCS/CONVERSIONS.md"; do
  set -- $pair
  generated="$(cargo run --quiet --release -- paths "$1")"
  if ! diff <(printf '%s\n' "$generated") "$2"; then
    echo "$2 is stale. Run: cargo run --release -- paths $1 > $2" >&2
    stale=1
  fi
done
[ "$stale" = 0 ] || exit 1
echo "DOCS/FORMATS.md and DOCS/CONVERSIONS.md are up to date"
