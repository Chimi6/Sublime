#!/bin/bash
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
out="$SC/w2p.txt"; : > "$out"
for f in "$ROOT"/tests/fixtures/pages/sources/*.docx; do "$HERE/w2p.sh" "$f" "f_$(basename "$f" .docx)" >> "$out" 2>&1; done
i=0; find "$CORPUS" -maxdepth 1 -name "*.docx" ! -name '~$*' -print0 | sort -z | while IFS= read -r -d '' f; do i=$((i+1)); "$HERE/w2p.sh" "$f" "u$i" | sed "s|^|$(basename "$f" | cut -c1-30) :: |" >> "$out"; done
for f in form_footnotes bug57031 drawing shapes-with-text bug59058 bib-chernigovka IllustrativeCases 60316 table-indent checkboxes 60329 WordWithAttachments table-alignment delins chartex issue_51265_1 Bug54849 sample issue_51265_3 headerFooter Bug51170 bug65738 VariousPictures Headers Bug54771a PageSpecificHeadFoot DiffFirstPageHeadFoot 60293 61745; do
  src=$(ls "$SC/web/poi/$f"*.docx 2>/dev/null | head -1); [ -n "$src" ] && "$HERE/w2p.sh" "$src" "p_$(echo $f | tr -c 'A-Za-z0-9\n' '_' | cut -c1-24)" >> "$out"; done
echo DONE >> "$out"
