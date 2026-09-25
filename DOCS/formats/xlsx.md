# Excel workbook (xlsx)

Office Open XML SpreadsheetML: a ZIP of XML parts. Sublime reads one
sheet of a workbook into rows (`src/io/xlsx/reader.rs`) and writes rows
into a one-sheet workbook (`src/io/xlsx/writer.rs`), since 0.16.0, so a
workbook reaches CSV, TSV, JSON, and JSON Lines through the row
converters and comes back from CSV and TSV. This is the living map of
what the reader and writer handle, tied to the tests that prove it.

## Status

- `xlsx -> csv`, `xlsx -> tsv`: shipped, conditional (one sheet;
  numbers as stored, dates as ISO 8601, booleans as `TRUE` and `FALSE`,
  formulas as their last value; formatting dropped). JSON and JSON Lines
  reach through them.
- `csv -> xlsx`, `tsv -> xlsx`: shipped, conditional (plain decimals of
  up to fifteen digits become numbers, everything else text; one sheet).
- `--sheet <name|number>` picks the sheet read (the first when absent)
  or names the sheet written (`Sheet1` when absent).

Oracles (`tests/xlsx_rows.rs`, fixtures in `tests/fixtures/xlsx` built
by hand from the specification): every workbook's first sheet reads to
the CSV beside it byte for byte; a sheet chosen by name or number reads
to its own CSV and a missing sheet is refused with the sheet list; every
CSV fixture survives CSV to workbook to CSV; the written workbook
carries the sheet name it was given. The fixtures cover shared strings
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
arrive (256 KiB deflated parts with a data descriptor). Empty cells are
skipped; a cell that is a plain decimal of at most fifteen significant
digits (no leading zeros, no trailing zeros in the fraction, no sign
plus) is written as a number, since Excel would store it exactly;
everything else is an inline string with whitespace preserved, so no
shared string table is held. Sheet names are cleaned to what Excel
accepts (31 characters, none of `\ / ? * [ ] :`).

## Known deviations

- One sheet per conversion; a workbook's other sheets need another run
  with `--sheet`. Reading every sheet at once is a roadmap item.
- Merged cells read as their top-left value with empties elsewhere;
  hidden rows and columns are read like any other.
- Number formats other than dates are dropped: `0.50` stored as `0.5`
  reads as `0.5`, currency and percent formats show their raw number.
- The reader inflates the sheet part whole before parsing it; a
  windowed inflate is the lever if very large sheets matter
  (`DOCS/benchmarks/xlsx-csv.md`).
- Excel-made fixtures are not yet in the set; the Mac session can add
  them, and the reader should be checked against them.

## Performance

`DOCS/benchmarks/xlsx-csv.md`.
