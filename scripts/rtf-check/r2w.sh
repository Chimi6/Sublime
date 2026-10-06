#!/bin/bash
# r2w.sh <lo|cocoa>: RTF -> Word. Each RTF of the flavor is rendered as the
# reference (LibreOffice for its own RTF; the macOS text system, as TextEdit
# lays it out, for Cocoa RTF), and our Word output and Apple's (textutil) are
# rendered by LibreOffice. Prints page counts and word recall against the
# reference, one line per file, into r2w-<flavor>.txt, then a summary.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${RTF_CHECK_DIR:-${TMPDIR:-/tmp}/rtf-check}"; B="$ROOT/target/release/sublime"; flavor="$1"
[ -x "$SC/cocoa-render" ] || swiftc -O "$HERE/cocoa-render.swift" -o "$SC/cocoa-render"
out="$SC/r2w-$flavor"; rm -rf "$out"; mkdir -p "$out/ref" "$out/ours" "$out/apple" "$out/pdf"
for f in "$SC/$flavor"/*.rtf; do
  n=$(basename "$f" .rtf)
  cp "$f" "$out/ref/$n.rtf"
  "$B" -q convert "$f" "$out/ours/$n.docx" 2>/dev/null
  textutil -convert docx "$f" -output "$out/apple/$n.docx" 2>/dev/null
done
for kind in ref ours apple; do
  mkdir -p "$out/pdf/$kind"
  if [ "$kind" = ref ] && [ "$flavor" = cocoa ]; then
    for f in "$out/ref"/*.rtf; do "$SC/cocoa-render" "$f" "$out/pdf/ref/$(basename "$f" .rtf).pdf" >/dev/null 2>&1; done
  else
    (cd "$out/$kind" && soffice --headless --convert-to pdf --outdir "$out/pdf/$kind" * >/dev/null 2>&1)
  fi
done
for f in "$SC/$flavor"/*.rtf; do
  n=$(basename "$f" .rtf)
  for kind in ref ours apple; do
    p="$out/pdf/$kind/$n.pdf"; [ -f "$p" ] && "$B" -q convert "$p" "$out/pdf/$kind/$n.txt" 2>/dev/null
  done
  python3 "$HERE/score.py" "$out/pdf" "$n" ref ours apple
done > "$SC/r2w-$flavor.txt"
python3 "$HERE/summary.py" "$SC/r2w-$flavor.txt"
