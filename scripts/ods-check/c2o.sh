#!/bin/bash
# c2o.sh DIR: CSV -> ODS by sublime for every CSV in DIR/ref (LibreOffice's
# exports, real cells of every kind), then LibreOffice's CSV export of each
# ODS we wrote (DIR/back/), which must read back as the CSV it came from.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
DIR="$1"; rm -rf "$DIR/w" "$DIR/back"; mkdir -p "$DIR/w" "$DIR/back"
for f in "$DIR"/ref/*.csv; do
  n=$(basename "$f" .csv)
  "$ROOT/target/release/sublime" -q convert "$f" "$DIR/w/$n.ods" 2>/dev/null || echo "failed: $n"
done
soffice --headless --convert-to 'csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,1' \
  --outdir "$DIR/back" "$DIR"/w/*.ods > "$DIR/back.log" 2>&1
python3 - "$DIR" <<'PY'
import csv, sys
from pathlib import Path
d = Path(sys.argv[1])
def rows(p):
    # Blank lines are skipped (a CSV reader cannot tell them from a row of
    # one empty cell), and LibreOffice's export starts at its used area.
    with open(p, encoding="utf-8", errors="replace", newline="") as h:
        out = [r for r in csv.reader(h) if any(r)]
    lead = min([next(i for i, v in enumerate(r) if v) for r in out] or [0])
    return [r[lead:] for r in out]
files = exact = cells = matched = 0
bad = []
for src in sorted((d / "ref").glob("*.csv")):
    back = d / "back" / f"{src.stem}-Sheet1.csv"
    files += 1
    a = rows(src); b = rows(back) if back.exists() else []
    if back.exists() and [x + [""] * (len(y) - len(x)) for x, y in zip(a, b)] == [y + [""] * (len(x) - len(y)) for x, y in zip(a, b)] and len(a) == len(b):
        exact += 1
    w = max([len(r) for r in a + b] or [0])
    for r in range(max(len(a), len(b))):
        x = a[r] if r < len(a) else []; y = b[r] if r < len(b) else []
        for c in range(w):
            u = x[c] if c < len(x) else ""; v = y[c] if c < len(y) else ""
            if u == "" and v == "":
                continue
            cells += 1; matched += u == v
            if u != v and len(bad) < 40:
                bad.append((src.stem, r + 1, c + 1, u, v))
print(f"files: {files}, read back the same: {exact}")
print(f"cells: {matched}/{cells} = {matched / max(cells, 1):.4f}")
for b in bad: print("  ", b)
PY
