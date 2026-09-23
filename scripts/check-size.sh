#!/usr/bin/env bash
# Fails when the release binary is larger than the byte count in size-budget.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --quiet
binary="target/release/sublime"
if [ -f "${binary}.exe" ]; then
  binary="${binary}.exe"
fi
actual="$(wc -c < "$binary" | tr -d ' ')"
budget="$(tr -d '[:space:]' < size-budget)"
echo "release binary: ${actual} bytes (budget ${budget})"
if [ "$actual" -gt "$budget" ]; then
  echo "binary exceeds size budget" >&2
  exit 1
fi
