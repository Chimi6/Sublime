#!/usr/bin/env bash
# Fails when the WebAssembly crate's version differs from the main crate's:
# both ship from the same tag.
set -euo pipefail
cd "$(dirname "$0")/.."
main="$(grep -m1 '^version = ' Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
wasm="$(grep -m1 '^version = ' wasm/Cargo.toml | sed -E 's/version = "(.*)"/\1/')"
echo "sublime ${main}, sublime-wasm ${wasm}"
if [ "$main" != "$wasm" ]; then
  echo "wasm/Cargo.toml version does not match Cargo.toml" >&2
  exit 1
fi
