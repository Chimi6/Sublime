#!/bin/bash
# corpus.sh: builds the RTF corpus under $RTF_CHECK_DIR from Word files: the
# repository's Word fixtures and every .docx in $RTF_CHECK_CORPUS (default
# ~/Downloads), each saved as RTF by LibreOffice (lo/) and by macOS's text
# system through textutil (cocoa/).
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${RTF_CHECK_DIR:-${TMPDIR:-/tmp}/rtf-check}"; CORPUS="${RTF_CHECK_CORPUS:-$HOME/Downloads}"
rm -rf "$SC/src" "$SC/lo" "$SC/cocoa"; mkdir -p "$SC/src" "$SC/lo" "$SC/cocoa"
i=0
{ find "$ROOT/tests/fixtures" -name "*.docx"; find "$CORPUS" -maxdepth 1 -name "*.docx" ! -name '~$*'; } | while IFS= read -r f; do
  i=$((i+1)); base=$(basename "$f" .docx | tr -c 'A-Za-z0-9_\n-' '_' | cut -c1-40)
  cp "$f" "$SC/src/$(printf %03d $i)-$base.docx"
done
(cd "$SC/src" && soffice --headless --convert-to rtf --outdir "$SC/lo" *.docx >/dev/null 2>&1)
for f in "$SC/src"/*.docx; do
  n=$(basename "$f" .docx)
  textutil -convert rtf "$f" -output "$SC/cocoa/$n.rtf" 2>/dev/null
done
echo "corpus: $(ls "$SC/src" | wc -l | tr -d ' ') Word files in $SC"
