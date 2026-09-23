#!/usr/bin/env bash
# Prints the CHANGELOG section for a version, for release notes.
# Usage: scripts/changelog-section.sh 0.1.0
set -euo pipefail
cd "$(dirname "$0")/.."
version="$1"
awk -v version="$version" '
  $0 ~ "^## \\[" version "\\]" { printing = 1; next }
  printing && /^## \[/ { exit }
  printing { print }
' DOCS/CHANGELOG.md
