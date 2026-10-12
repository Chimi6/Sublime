"""same.py <dir>: whether Numbers' CSV export of our file (<dir>/app, a file
or a folder of `Sheet-Table.csv`) matches our reader's CSV of the same file
(<dir>/read.csv or <dir>/read/<part>.csv), table by table in order, cell by
cell, trailing empty cells aside. Prints `same` or what differs."""
import csv
import glob
import io
import os
import sys

directory = sys.argv[1]


def rows(path):
    text = open(path, encoding="utf-8-sig", newline="").read()
    out = []
    for row in csv.reader(io.StringIO(text)):
        while row and row[-1] == "":
            row.pop()
        out.append(row)
    while out and not out[-1]:
        out.pop()
    return out


app = os.path.join(directory, "app")
read = os.path.join(directory, "read")
if os.path.isdir(read):
    # Written through a workbook, every sheet holds one table, `Table 1`:
    # our part `Sheet` is Numbers' `Sheet-Table 1.csv`.
    pairs = [
        (os.path.join(app, os.path.basename(our)[:-4] + "-Table 1.csv"), our)
        for our in sorted(glob.glob(os.path.join(read, "*.csv")))
    ]
else:
    pairs = [(app, read + ".csv")]
missing = [path for pair in pairs for path in pair if not os.path.exists(path)]
if missing or (os.path.isdir(app) and len(glob.glob(os.path.join(app, "*.csv"))) != len(pairs)):
    print(f"tables differ ({len(pairs)} ours; missing {[os.path.basename(path) for path in missing][:2]})")
    sys.exit()
for their, our in pairs:
    want, got = rows(their), rows(our)
    if want != got:
        for i, (a, b) in enumerate(zip(want, got)):
            if a != b:
                print(f"differs in {os.path.basename(their)} row {i}: {a[:4]} vs {b[:4]}")
                sys.exit()
        print(f"differs in {os.path.basename(their)}: {len(want)} vs {len(got)} rows")
        sys.exit()
print("same")
