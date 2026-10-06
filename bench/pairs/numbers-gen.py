"""numbers-gen.py <shape> <rows> <out.numbers>: a Numbers document written
by numbers-parser, the bench input for the numbers-csv pair.

dense:  one table of <rows> rows and eight columns of mixed values (text,
        integers, decimals, dates, booleans), as an export of records.
sheets: twenty sheets of three tables each, <rows> rows across them all.
"""
import datetime
import random
import sys

from numbers_parser import Document

COLUMNS = ["id", "name", "city", "amount", "quantity", "date", "active", "note"]
CITIES = ["Lisbon", "Osaka", "Denver", "Nairobi", "Oslo", "Quito", "Perth", "Seoul"]
WORDS = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu".split()


def record(index, rng):
    return [
        index,
        f"{rng.choice(WORDS).title()} {rng.choice(WORDS).title()}",
        rng.choice(CITIES),
        round(rng.uniform(0, 10000), 2),
        rng.randint(1, 500),
        datetime.datetime(2020, 1, 1) + datetime.timedelta(days=rng.randint(0, 2000)),
        rng.random() < 0.5,
        " ".join(rng.choice(WORDS) for _ in range(rng.randint(2, 8))),
    ]


def fill(table, rows, start, rng):
    for column, name in enumerate(COLUMNS):
        table.write(0, column, name)
    for row in range(rows):
        for column, value in enumerate(record(start + row, rng)):
            table.write(row + 1, column, value)


def main():
    shape, rows, out = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    rng = random.Random(7)
    document = Document(num_rows=2, num_cols=len(COLUMNS))
    if shape == "dense":
        fill(document.sheets[0].tables[0], rows, 0, rng)
    else:
        per_table = max(rows // 60, 1)
        for sheet_index in range(20):
            if sheet_index > 0:
                document.add_sheet(f"Sheet {sheet_index + 1}", num_rows=2, num_cols=len(COLUMNS))
            sheet = document.sheets[sheet_index]
            for table_index in range(3):
                if table_index > 0:
                    sheet.add_table(f"Table {table_index + 1}", num_rows=2, num_cols=len(COLUMNS))
                fill(sheet.tables[table_index], per_table, (sheet_index * 3 + table_index) * per_table, rng)
    document.save(out)


if __name__ == "__main__":
    main()
