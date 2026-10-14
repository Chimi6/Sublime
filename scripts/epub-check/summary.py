#!/usr/bin/env python3
"""Sums score.py lines: mean recall and precision per tool, and how many
files each keeps every note, picture, table, and heading of.

    summary.py LINES [CHECKS]

CHECKS, when given, is epubcheck's output per tool (lines `tool file
ERROR...`), counted as files with errors.
"""

import collections
import re
import sys


def main():
    stats = collections.defaultdict(lambda: collections.defaultdict(list))
    files = 0
    for line in open(sys.argv[1]):
        files += 1
        for tool, recall, precision, counts in re.findall(
            r"(\w+) recall=([\d.]+) precision=([\d.]+) counts=(\S+)", line
        ):
            stats[tool]["recall"].append(float(recall))
            stats[tool]["precision"].append(float(precision))
            if counts == "missing":
                for key in ("notes", "images", "tables", "headings"):
                    stats[tool][key].append(False)
                continue
            for key, pair in zip(("notes", "images", "tables", "headings"), counts.split(",")):
                got, want = (int(n) for n in pair.split("/"))
                stats[tool][key].append(got == want)
    errors = collections.Counter()
    if len(sys.argv) > 2:
        seen = set()
        for line in open(sys.argv[2]):
            tool, name, level = line.split()[:3]
            if level.startswith(("ERROR", "FATAL")) and (tool, name) not in seen:
                seen.add((tool, name))
                errors[tool] += 1
    print(f"files {files}")
    for tool, values in stats.items():
        mean = lambda key: sum(values[key]) / max(1, len(values[key]))
        kept = " ".join(f"{key} {sum(values[key])}" for key in ("notes", "images", "tables", "headings"))
        line = f"  {tool:7} recall {mean('recall'):.3f} precision {mean('precision'):.3f}  all kept: {kept}"
        if len(sys.argv) > 2:
            line += f"  epubcheck errors in {errors[tool]} files"
        print(line)


if __name__ == "__main__":
    main()
