"""numbers-parser-csv.py <in.numbers> <out-dir>: the reference for the
numbers-csv pair. Reads every table with numbers-parser and writes each as
a CSV with Python's csv module, as `sublime` writes a CSV per table."""
import csv
import os
import sys

from numbers_parser import Document

document = Document(sys.argv[1])
os.makedirs(sys.argv[2], exist_ok=True)
for sheet in document.sheets:
    for table in sheet.tables:
        name = f"{sheet.name} - {table.name}".replace("/", "_")
        with open(os.path.join(sys.argv[2], name + ".csv"), "w", newline="") as handle:
            writer = csv.writer(handle)
            for row in table.rows(values_only=True):
                writer.writerow(row)
