#!/usr/bin/env python3
"""diff.py REF.csv OURS.csv [N]: the first N cells that differ."""
import csv, sys
def rows(p):
    with open(p, encoding="utf-8", errors="replace", newline="") as h:
        return list(csv.reader(h))
a, b = rows(sys.argv[1]), rows(sys.argv[2])
n = int(sys.argv[3]) if len(sys.argv) > 3 else 20
print(f"ref {len(a)} rows x {max(map(len, a or [[]]))}, ours {len(b)} rows x {max(map(len, b or [[]]))}")
for r in range(max(len(a), len(b))):
    x = a[r] if r < len(a) else []
    y = b[r] if r < len(b) else []
    for c in range(max(len(x), len(y))):
        u = x[c] if c < len(x) else ""
        v = y[c] if c < len(y) else ""
        if u != v:
            print(f"  r{r+1} c{c+1}: ref {u!r} ours {v!r}")
            n -= 1
            if n == 0:
                sys.exit()
