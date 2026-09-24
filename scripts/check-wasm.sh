#!/usr/bin/env bash
# Builds the WebAssembly module, runs its smoke test under node, and fails
# when the module is larger than the byte count in wasm/size-budget.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --quiet --target wasm32-unknown-unknown --manifest-path wasm/Cargo.toml
module="wasm/target/wasm32-unknown-unknown/release/sublime_wasm.wasm"
node wasm/smoke.mjs
actual="$(wc -c < "$module" | tr -d ' ')"
budget="$(tr -d '[:space:]' < wasm/size-budget)"
echo "wasm module: ${actual} bytes (budget ${budget})"
if [ "$actual" -gt "$budget" ]; then
  echo "wasm module exceeds size budget" >&2
  exit 1
fi
