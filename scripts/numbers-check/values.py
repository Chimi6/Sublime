"""values.py <in.numbers> <out.json>: every table's cell values as
numbers-parser reads them, as text the way ours writes them: dates in ISO
8601, booleans as TRUE and FALSE, durations as seconds (`dur:N`)."""
import datetime
import json
import sys

from numbers_parser import Document


def text(value):
    if value is None:
        return ""
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, datetime.datetime):
        return value.isoformat(sep=" ")
    if isinstance(value, datetime.timedelta):
        return f"dur:{value.total_seconds()}"
    return str(value)


document = Document(sys.argv[1])
out = {"sheets": []}
for sheet in document.sheets:
    entry = {"name": sheet.name, "tables": []}
    for table in sheet.tables:
        rows = []
        for row in table.rows():
            cells = []
            for cell in row:
                try:
                    cells.append(text(cell.value))
                except Exception:  # noqa: BLE001
                    cells.append("")
            rows.append(cells)
        entry["tables"].append({"name": table.name, "rows": rows})
    out["sheets"].append(entry)
with open(sys.argv[2], "w") as handle:
    json.dump(out, handle, ensure_ascii=False)
