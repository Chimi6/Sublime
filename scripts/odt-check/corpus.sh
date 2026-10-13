#!/bin/bash
# corpus.sh: builds the ODT corpus under $ODT_CHECK_DIR from Word files: the
# repository's Word fixtures and every .docx in $ODT_CHECK_CORPUS (default
# ~/Downloads), each saved as ODT by LibreOffice (lo/) and by pandoc (pandoc/).
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${ODT_CHECK_DIR:-${TMPDIR:-/tmp}/odt-check}"; CORPUS="${ODT_CHECK_CORPUS:-$HOME/Downloads}"
rm -rf "$SC/src" "$SC/lo" "$SC/pandoc"; mkdir -p "$SC/src" "$SC/lo" "$SC/pandoc"
i=0
{ find "$ROOT/tests/fixtures" -name "*.docx"; find "$CORPUS" -maxdepth 1 -name "*.docx" ! -name '~$*'; } | while IFS= read -r f; do
  i=$((i+1)); base=$(basename "$f" .docx | tr -c 'A-Za-z0-9_\n-' '_' | cut -c1-40)
  cp "$f" "$SC/src/$(printf %03d $i)-$base.docx"
done
(cd "$SC/src" && soffice --headless --convert-to odt --outdir "$SC/lo" *.docx >/dev/null 2>&1)
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  pandoc "$f" -o "$SC/pandoc/$n.odt" 2>/dev/null
done
echo "corpus: $(ls "$SC/src" | wc -l | tr -d ' ') Word files in $SC"
