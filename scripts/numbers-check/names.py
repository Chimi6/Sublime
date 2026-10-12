"""names.py <in.numbers> <out.json>: a document's sheet and table names, in
order, as numbers-parser reads them, to pair Numbers' export files with
our parts."""
import json
import sys

from numbers_parser import Document

document = Document(sys.argv[1])
names = [{"name": sheet.name, "tables": [table.name for table in sheet.tables]} for sheet in document.sheets]
with open(sys.argv[2], "w") as handle:
    json.dump(names, handle, ensure_ascii=False)
