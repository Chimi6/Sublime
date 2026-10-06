#!/usr/bin/env python3
"""summary.py <lines>: for each kind, how many files match the reference's
page count and the mean recall."""
import sys
from collections import defaultdict

matches, recalls, files = defaultdict(int), defaultdict(float), 0
for line in open(sys.argv[1]):
    parts = line.split()
    if "pages" not in parts:
        continue
    files += 1
    at = parts.index("pages")
    stop = parts.index("recall")
    counts = dict(part.split("=") for part in parts[at + 1 : stop])
    reference = parts[at + 1].split("=")[0]
    for kind, value in counts.items():
        if kind != reference and value == counts[reference]:
            matches[kind] += 1
    for part in parts[stop + 1 :]:
        kind, value = part.split("=")
        recalls[kind] += float(value)
kinds = [kind for kind in recalls]
print(f"files {files}")
for kind in kinds:
    print(f"  {kind:6} page count matches {matches[kind]:3}  mean recall {recalls[kind] / max(files, 1):.3f}")
