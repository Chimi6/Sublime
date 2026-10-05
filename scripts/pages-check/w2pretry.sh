#!/bin/bash
# w2pretry.sh: reruns the w2p rows whose Apple import failed (apple=0) and replaces their lines.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
out="$SC/w2p.txt"
names=$(grep "apple=0" "$out" | sed -E 's/^(.*:: )?([^:]+): pages.*/\2/')
i=0; declare -a user
while IFS= read -r -d '' f; do i=$((i+1)); user[$i]="$f"; done < <(find "$CORPUS" -maxdepth 1 -name "*.docx" ! -name '~$*' -print0 | sort -z)
for n in $names; do
  case "$n" in
    f_*) src="$ROOT/tests/fixtures/pages/sources/${n#f_}.docx";;
    u*) src="${user[${n#u}]}";;
    p_*) key="${n#p_}"; src=$(ls "$SC"/web/poi/*.docx | while read -r c; do b=$(basename "$c" .docx | tr -c 'A-Za-z0-9\n' '_' | cut -c1-24); [ "$b" = "$key" ] && echo "$c"; done | head -1);;
  esac
  [ -f "$src" ] || { echo "no source for $n"; continue; }
  for attempt in 1 2; do
    line=$("$HERE/w2p.sh" "$src" "$n" | tail -1)
    echo "$line" | grep -q "apple=0" || break
  done
  prefix=$(grep -E "(^|:: )$n: pages" "$out" | head -1 | sed -E "s/$n: pages.*//")
  python3 - "$out" "$n" "$prefix$line" <<'PY'
import sys
path,name,new=sys.argv[1:4]
lines=open(path).read().split('\n')
for i,l in enumerate(lines):
    if l.startswith(name+': pages') or (':: '+name+': pages') in l:
        lines[i]=new; break
open(path,'w').write('\n'.join(lines))
PY
done
echo RETRY-DONE
