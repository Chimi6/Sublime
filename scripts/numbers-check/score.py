"""score.py <dir> [-v]: compares our CSVs (<dir>/ours/<name>/) with
numbers-parser's values (<dir>/ref/<name>.json), cell by cell. Numbers
compare as numbers; a date compares as its day, and as its day and time
(`YYYY-MM-DDTHH:MM:SS`) when it has a time. Prints the files that match exactly and the share of
non-empty cells that match; -v lists each file's first differences."""
import csv
import glob
import json
import os
import re
import sys

directory = sys.argv[1]
verbose = "-v" in sys.argv[2:]


def number(text):
    try:
        return float(text)
    except ValueError:
        return None


def same(want, got):
    if want == got:
        return True
    if got is None:
        return False
    a, b = number(want), number(got)
    if a is not None and b is not None:
        return a == b or abs(a - b) <= 1e-9 * max(abs(a), abs(b))
    date = re.match(r"(\d{4}-\d\d-\d\d) (\d\d:\d\d:\d\d)", want)
    if date:
        midnight = date.group(2) == "00:00:00"
        return got == (date.group(1) if midnight else f"{date.group(1)}T{date.group(2)}")
    return False


def safe(name):
    return re.sub(r'[/\\:*?"<>|]', "_", name)


files = exact = cells = matched = 0
for path in sorted(glob.glob(os.path.join(directory, "ref", "*.json"))):
    name = os.path.basename(path)[:-5]
    sheets = json.load(open(path))["sheets"]
    tables = [
        (s["name"] if len(s["tables"]) == 1 else f'{s["name"]} - {t["name"]}', t["rows"])
        for s in sheets
        for t in s["tables"]
    ]
    ours = os.path.join(directory, "ours", name)
    count = good = 0
    differences = []
    for table, rows in tables:
        single = os.path.join(ours, "out.csv")
        csv_path = single if os.path.exists(single) else os.path.join(ours, "out", safe(table) + ".csv")
        got = list(csv.reader(open(csv_path, newline=""))) if os.path.exists(csv_path) else []
        for i, row in enumerate(rows):
            for j, want in enumerate(row):
                if want == "":
                    continue
                count += 1
                value = got[i][j] if i < len(got) and j < len(got[i]) else None
                if same(want, value):
                    good += 1
                else:
                    differences.append((table, i, j, want[:40], (value or "")[:40]))
    files += 1
    cells += count
    matched += good
    if good == count:
        exact += 1
    elif verbose:
        print(name, f"{good}/{count}", differences[:4])
print(f"{exact}/{files} files exact, {matched}/{cells} cells ({matched / max(cells, 1):.4f})")
