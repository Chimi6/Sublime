"""pages-xlsx-gen.py <shape> <rows> <out.md>: Markdown for the pages-xlsx
pair, written to Pages by Sublime's own writer (no large Pages document
with tables can be committed).

tables: tables of 200 rows each, a heading over each, <rows> rows in all.
report: a report: four paragraphs of prose, then a heading and a 25-row
        table, repeated until <rows> table rows.
Cells mix text, decimals, ISO dates, and TRUE and FALSE.
"""
import datetime
import random
import sys

WORDS = "alpha beta gamma delta epsilon zeta eta theta iota kappa lambda mu".split()
CITIES = ["Lisbon", "Osaka", "Denver", "Nairobi", "Oslo", "Quito", "Perth", "Seoul"]


def table(out, rng, start, rows):
    out.write("| id | name | city | amount | date | active |\n")
    out.write("| --- | --- | --- | --- | --- | --- |\n")
    for index in range(start, start + rows):
        date = datetime.date(2020, 1, 1) + datetime.timedelta(days=rng.randint(0, 2000))
        out.write(
            f"| {index} | {rng.choice(WORDS).title()} {rng.choice(WORDS).title()} | "
            f"{rng.choice(CITIES)} | {rng.randint(0, 999999) / 100} | {date.isoformat()} | "
            f"{'TRUE' if rng.random() < 0.5 else 'FALSE'} |\n"
        )
    out.write("\n")


def main():
    shape, rows, path = sys.argv[1], int(sys.argv[2]), sys.argv[3]
    rng = random.Random(11)
    per_table = 200 if shape == "tables" else 25
    with open(path, "w") as out:
        written = 0
        number = 0
        while written < rows:
            number += 1
            if shape == "report":
                for _ in range(4):
                    out.write(" ".join(rng.choice(WORDS) for _ in range(rng.randint(40, 90))).capitalize() + ".\n\n")
            out.write(f"## Table {number}\n\n")
            count = min(per_table, rows - written)
            table(out, rng, written, count)
            written += count


if __name__ == "__main__":
    main()
