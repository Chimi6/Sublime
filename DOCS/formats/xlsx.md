# Excel workbook (xlsx)

Office Open XML SpreadsheetML: a ZIP of XML parts. Sublime reads every
sheet of a workbook into rows (`src/io/xlsx/reader.rs`) and writes rows
into a workbook of one or more sheets (`src/io/xlsx/writer.rs`), so a
workbook reaches CSV, TSV, JSON, JSON Lines, and every document format,
whole. This is the living map of what the reader and writer handle, tied
to the tests that prove it.

## Workbooks whole

A workbook is several tables. Where the target holds one table, each sheet
becomes its own file; where it holds several, the workbook stays one file:

| Target | A workbook of several sheets becomes |
|---|---|
| CSV, TSV, JSON Lines | a folder named after the output, a file per sheet (`book.csv` -> `book/Sales.csv`, `book/Costs.csv`); one sheet is one file, as before |
| JSON (and YAML, TOML, XML through it) | one object of arrays keyed by sheet name; one sheet stays the plain array |
| Markdown, HTML, Word, Pages, RTF, PDF | one document, each sheet a table under a heading of its name |
| standard output | the first sheet, the rest reported as a loss |
| the WebAssembly module | a ZIP of the files the command line would write |

The other way, a workbook is built whole: a document's tables become a
sheet each, named after the heading before each (`markdown-to-xlsx`, and
Word, Pages, HTML, and PDF through it), and a JSON object of arrays a
sheet per member (`json-to-xlsx`). The same split serves documents: a
document's tables into CSV are a folder of CSVs, and into JSON one object
of arrays (`markdown-tables-to-json`).

`--sheet <name|number>` still picks one sheet (one file) or names the
sheet a single table is written to (`Sheet1` when absent).

How it works: a converter that can split (`Converter::splits`) writes each
part through `Converter::convert_parts`; the planner splits only when the
plan ends in a format that holds one part (`Format::holds_one_part`) and
runs the hops after the split once per part (`planner::execute_parts`);
the command line moves the parts into place, and the module zips them.

## Status

- `xlsx -> csv`, `xlsx -> tsv`: shipped, conditional (a file per sheet,
  or the one `--sheet` picks; numbers as stored, dates as ISO 8601,
  booleans as `TRUE` and `FALSE`, formulas as their last value;
  formatting dropped). JSON Lines reaches through them.
- `xlsx -> json`, `xlsx -> markdown`: shipped, conditional (every sheet
  in one file, as above).
- `csv -> xlsx`, `tsv -> xlsx`: shipped, conditional (plain decimals of
  up to fifteen digits become numbers, everything else text; one sheet).
- `markdown -> xlsx`, `json -> xlsx`: shipped (a sheet per table or per
  member). Empty cells are written as cells without a value, so a row
  read back keeps its width.

Oracles (`tests/xlsx_rows.rs`, `tests/workbooks.rs`, fixtures in
`tests/fixtures/xlsx` built by hand from the specification): every
workbook's first sheet reads to the CSV beside it byte for byte; a sheet
chosen by name or number reads to its own CSV and a missing sheet is
refused with the sheet list; every sheet of a two-sheet workbook reads to
its own CSV; a workbook survives JSON and back; a document's three tables
make three named sheets and three CSVs; every CSV fixture survives CSV to
workbook to CSV; the written workbook carries the sheet name it was
given. The fixtures cover shared strings
(plain, rich text runs, phonetic runs skipped, preserved whitespace,
entities), inline strings, numbers as stored (`1E-05`), dates and
datetimes by built-in and custom number formats, times, booleans,
formula cells with cached values, error cells, skipped rows and cells,
the sheet dimension, the 1904 epoch, a workbook without shared strings
or styles, an empty sheet, and relationship ids out of order.

## What the reader does

| Part | Read |
|---|---|
| `xl/workbook.xml` and its `.rels` | the sheet list in order, each name joined to its part through its relationship id; `date1904` |
| `xl/sharedStrings.xml` | every `si` as its `t` runs joined, phonetic runs (`rPh`) left out |
| `xl/styles.xml` | per `cellXfs` entry, whether its number format is a date: the built-in date ids (14 to 22, 27 to 36, 45 to 47, 50 to 58) or a custom `formatCode` with a day, month, year, hour, or second token outside quotes and brackets |
| the sheet's `sheetData` | rows in order, each cell placed at its column (`r`), gaps filled with empty cells, rows padded to the `dimension` width, skipped row numbers emitted as empty rows |

Cell text by type: `s` the shared string; `inlineStr` and `str` the
text; `b` `TRUE` or `FALSE`; `e` the error code; `n` (or no type) the
value as stored, or, when the cell's style is a date format, the serial
as `YYYY-MM-DD`, `YYYY-MM-DDTHH:MM:SS`, or `HH:MM:SS` (seconds rounded).
Serials follow the 1900 system with its February 29 quirk, or the 1904
system when the workbook says so.

## What the writer does

A package of six parts: content types, package relationships,
`workbook.xml` with one sheet, its relationships, a minimal
`styles.xml`, and the worksheet, which streams into the ZIP as rows
arrive (256 KiB deflated parts with a data descriptor). A cell is typed
only when reading it back gives the same text (the rule is tested by
writing and reading back a set of near misses):

| Cell text | Written as |
|---|---|
| a plain decimal of at most fifteen significant digits (no leading zeros, no trailing zeros in the fraction, no sign plus) | a number, which Excel stores exactly |
| an ISO 8601 date (`2024-08-08`), date and time (`2024-08-08T14:35:09`, not midnight), or time (`14:35:09`), from 1900-03-01 to 9999-12-31 | a date serial with a style that shows it in the same form (`yyyy-mm-dd`, `yyyy-mm-dd"T"hh:mm:ss`, `[hh]:mm:ss`) |
| `TRUE` or `FALSE` | a boolean |
| empty | a cell without a value, so a row keeps its width |
| anything else | an inline string with whitespace preserved, so no shared string table is held |

These are the forms the readers write (Excel's and Numbers' dates as ISO
8601, booleans as `TRUE` and `FALSE`), so a workbook's dates and booleans
survive any path through rows. It is a far narrower set than Excel or
LibreOffice convert when they open a CSV: locale dates (`3/4`), partial
dates (`1-2`, `SEPT2`), and lowercase `true` stay text. The one change in
kind: a source cell stored as text that is exactly an ISO date or `TRUE`
becomes a date or boolean, shown the same way. Sheet names are cleaned to what Excel
accepts (31 characters, none of `\ / ? * [ ] :`).

## Known deviations

- Merged cells read as their top-left value with empties elsewhere;
  hidden rows and columns are read like any other.
- Number formats other than dates are dropped: `0.50` stored as `0.5`
  reads as `0.5`, currency and percent formats show their raw number.
- Cells pass between readers and the writer as text, so the writer types
  them by their form (above), not by the type the source gave them.
  Typed cells end to end are recorded in `DOCS/STATE.md` (Future).
- The reader inflates the sheet part whole before parsing it; a
  windowed inflate is the lever if very large sheets matter
  (`DOCS/benchmarks/xlsx-csv.md`).
- Excel-made fixtures are not yet in the set; the Mac session can add
  them, and the reader should be checked against them.

## Performance

`DOCS/benchmarks/xlsx-csv.md`.
