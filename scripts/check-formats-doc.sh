#!/usr/bin/env bash
# Fails when DOCS/FORMATS.md does not match the registry.
set -euo pipefail
cd "$(dirname "$0")/.."
generated="$(cargo run --quiet --release -- paths --markdown)"
if ! diff <(printf '%s\n' "$generated") DOCS/FORMATS.md; then
  echo "DOCS/FORMATS.md is stale. Run: cargo run --release -- paths --markdown > DOCS/FORMATS.md" >&2
  exit 1
fi
echo "DOCS/FORMATS.md is up to date"
