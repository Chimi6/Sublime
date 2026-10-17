#!/usr/bin/env python3
"""score.py REF OUT SRC: each sheet's CSV in OUT against LibreOffice's in REF.

REF holds <name>-<sheet>.csv (LibreOffice's export of every sheet); OUT holds
<name>.csv for a one-sheet spreadsheet or <name>/<sheet>.csv. Prints the
sheets that match byte for byte, the share of matching cells, and the worst
sheets."""

import csv
import sys
import zipfile
import xml.etree.ElementTree as ET
from pathlib import Path

TABLE = "{urn:oasis:names:tc:opendocument:xmlns:table:1.0}"


def formula_cells(path):
    """Per sheet name, the (row, column) of every formula cell: LibreOffice
    recalculates some on loading, so their text can differ for reasons no
    reader controls."""
    sheets = {}
    try:
        root = ET.fromstring(zipfile.ZipFile(path).read("content.xml"))
    except Exception:
        return sheets
    for table in root.iter(TABLE + "table"):
        cells = set()
        row = 0
        for element in table.iter():
            if element.tag == TABLE + "table-row":
                repeat = int(element.get(TABLE + "number-rows-repeated", "1"))
                column = 0
                for cell in element:
                    if cell.tag not in (TABLE + "table-cell", TABLE + "covered-table-cell"):
                        continue
                    span = int(cell.get(TABLE + "number-columns-repeated", "1"))
                    if cell.get(TABLE + "formula") is not None:
                        for r in range(row, row + min(repeat, 64)):
                            for c in range(column, column + min(span, 64)):
                                cells.add((r, c))
                    column += span
                row += repeat
        sheets[table.get(TABLE + "name")] = cells
    return sheets


def rows(path):
    with open(path, encoding="utf-8", errors="replace", newline="") as handle:
        return list(csv.reader(handle))


def main():
    ref, out, src = map(Path, sys.argv[1:4])
    names = sorted(path.stem for path in src.glob("*.ods"))
    total = exact = cells = matched = missing = 0
    plain_cells = plain_matched = 0
    worst = []
    for name in names:
        sheets = sorted(ref.glob(f"{name}-*.csv"))
        # A sheet name with the file name's prefix in it: keep the longest
        # file name that owns the export.
        sheets = [s for s in sheets if not any(
            other != name and other.startswith(name + "-") and s.stem.startswith(other + "-")
            for other in names)]
        flat = len(sys.argv) > 5 and sys.argv[5] == "flat"
        folder = out / name
        formulas = formula_cells(src / f"{name}.ods")
        for sheet in sheets:
            sheet_name = sheet.stem[len(name) + 1:]
            total += 1
            if flat:
                # Excel names hold 31 characters and no `/`.
                ours = out / sheet.name
                if not ours.exists():
                    short = sheet_name.replace("/", "~")[:31]
                    ours = out / f"{name}-{short}.csv"
            else:
                ours = folder / f"{sheet_name}.csv" if folder.is_dir() else out / f"{name}.csv"
            if not flat and folder.is_dir() and not ours.exists():
                candidates = [p for p in folder.glob("*.csv") if p.stem.replace(" ", "_") == sheet_name.replace(" ", "_")]
                ours = candidates[0] if candidates else ours
            reference = rows(sheet)
            if not ours.exists():
                missing += 1
                got = []
            else:
                got = rows(ours)
                if ours.read_bytes() == sheet.read_bytes():
                    exact += 1
            height = max(len(reference), len(got))
            width = max([len(r) for r in reference + got] or [0])
            sheet_cells = sheet_matched = 0
            sheet_formulas = formulas.get(sheet_name, set())
            for r in range(height):
                a = reference[r] if r < len(reference) else []
                b = got[r] if r < len(got) else []
                for c in range(width):
                    x = a[c] if c < len(a) else ""
                    y = b[c] if c < len(b) else ""
                    if x == "" and y == "":
                        continue
                    sheet_cells += 1
                    sheet_matched += x == y
                    if (r, c) not in sheet_formulas:
                        plain_cells += 1
                        plain_matched += x == y
            cells += sheet_cells
            matched += sheet_matched
            if sheet_matched < sheet_cells:
                worst.append((sheet_cells - sheet_matched, sheet_cells, f"{name} / {sheet_name}"))
    worst.sort(reverse=True)
    print(f"sheets: {total}, byte-identical: {exact}, missing: {missing}")
    print(f"cells: {matched}/{cells} = {matched / max(cells, 1):.4f}")
    print(f"cells without formulas: {plain_matched}/{plain_cells} = {plain_matched / max(plain_cells, 1):.4f}")
    for wrong, count, label in worst[:int(sys.argv[4]) if len(sys.argv) > 4 else 25]:
        print(f"  {wrong:6d} of {count:6d} wrong  {label}")


if __name__ == "__main__":
    main()
