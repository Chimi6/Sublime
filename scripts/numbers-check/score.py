"""score.py <dir> <ours|xl> [-v]: compares our tables with Numbers' own CSV
export (<dir>/app/<name>, a file, or a folder of `Sheet-Table.csv`), cell by
cell, for every non-empty cell Numbers shows. `ours` reads our CSV parts
(<dir>/ours/<name>/out.csv or out/<part>.csv); `xl` reads LibreOffice's CSV
of our workbook (<dir>/xl/<name>/out-<sheet>.csv), where a figure space
(Excel's `?` digit) counts as a space. Prints the documents that match
exactly and the share of cells; -v lists each document's first
differences."""
import csv
import glob
import io
import json
import os
import re
import sys

directory, kind = sys.argv[1], sys.argv[2]
verbose = "-v" in sys.argv[3:]


def rows(path):
    text = open(path, encoding="utf-8-sig", newline="").read().replace(" ", " ")
    return list(csv.reader(io.StringIO(text)))


def safe(name):
    return re.sub(r'[/\\:*?"<>|]', "_", name)


files = exact = cells = matched = 0
for names in sorted(glob.glob(os.path.join(directory, "names", "*.json"))):
    name = os.path.basename(names)[:-5]
    app = os.path.join(directory, "app", name)
    if not os.path.exists(app):
        continue
    sheets = json.load(open(names))
    count = good = 0
    differences = []
    for sheet in sheets:
        for table in sheet["tables"]:
            reference = os.path.join(app, f"{sheet['name']}-{table}.csv") if os.path.isdir(app) else app
            if not os.path.exists(reference):
                continue
            part = sheet["name"] if len(sheet["tables"]) == 1 else f"{sheet['name']} - {table}"
            if kind == "ours":
                single = os.path.join(directory, "ours", name, "out.csv")
                path = single if os.path.exists(single) else os.path.join(directory, "ours", name, "out", safe(part) + ".csv")
            else:
                path = os.path.join(directory, "xl", name, f"out-{part[:31]}.csv")
            got = rows(path) if os.path.exists(path) else []
            for i, row in enumerate(rows(reference)):
                for j, want in enumerate(row):
                    if want == "":
                        continue
                    count += 1
                    value = got[i][j] if i < len(got) and j < len(got[i]) else None
                    if value == want:
                        good += 1
                    else:
                        differences.append((part, i, j, want[:40], (value or "")[:40]))
    files += 1
    cells += count
    matched += good
    if good == count:
        exact += 1
    elif verbose:
        print(name, f"{good}/{count}", differences[:4])
print(f"{exact}/{files} documents exact, {matched}/{cells} cells ({matched / max(cells, 1):.4f})")
