#!/bin/bash
# e2e.sh <input.docx> <name> : convert with sublime, open in Pages, export PDF, render pages.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
in="$1"; name="$2"; out="$SC/out/$name.pages"; mkdir -p "$SC/out"
"$ROOT/target/release/sublime" convert "$in" "$out" -q || { echo "CONVERT-FAILED"; exit 4; }
rm -f "$SC/out/$name"-p*.png "$SC/out/$name.pdf"
"$HERE/pagestest.sh" "$out" "$SC/out/$name.pdf"; rc=$?
[ -f "$SC/out/$name.pdf" ] && "$SC/render" "$SC/out/$name.pdf" "$SC/out/$name"
exit $rc
