#!/usr/bin/env bash
# Fails when a non-dev runtime dependency is not listed in DOCS/DEPENDENCIES.md.
set -euo pipefail
cd "$(dirname "$0")/.."
dependencies="$(cargo tree --edges normal --prefix none --no-dedupe --quiet | awk '{print $1}' | grep -v '^sublime$' | sort -u || true)"
if [ -z "$dependencies" ]; then
  echo "no runtime dependencies"
  exit 0
fi
status=0
while IFS= read -r crate; do
  [ -z "$crate" ] && continue
  if ! grep -Eq "^\| ${crate} \|" DOCS/DEPENDENCIES.md; then
    echo "runtime dependency '${crate}' is not listed in DOCS/DEPENDENCIES.md" >&2
    status=1
  fi
done <<< "$dependencies"
exit "$status"
