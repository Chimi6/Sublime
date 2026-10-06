# Apple Numbers (numbers)

A Numbers document is an iWork package like Pages: a ZIP of IWA streams
(Snappy-framed protobuf objects). It is read through the Pages package
reader (`src/io/pages/package.rs`) and the Pages table reader, which shares
the table archives (TST) with Numbers. `io::pages::read_workbook` walks the
document's sheets (`TN.DocumentArchive`, `TN.SheetArchive`, read by field
number because the Pages registry leaves Numbers' own types out), each
sheet's drawables to its tables (`TST.TableInfoArchive`, through groups),
and each table's cells as text.

## Status

- `numbers -> csv`, `tsv`, `json`, `markdown`, `xlsx`: shipped, and every
  format those reach (JSON Lines, YAML, TOML, XML, Word, Pages, HTML, RTF,
  PDF).
- Writing Numbers: not yet. A writer needs the object-graph work the Pages
  writer did, for Numbers' archives.

## A workbook of tables

A Numbers sheet holds several tables, so a document is a list of tables
(see "Workbooks whole" in `xlsx.md`). Each table is named after its sheet
when the sheet holds only it, and `Sheet - Table` otherwise, as Numbers
names worksheets when it exports to Excel.

| Target | A document of several tables becomes |
|---|---|
| CSV, TSV, JSON Lines | a folder named after the output, a file per table; one table is one file |
| JSON | one object of arrays keyed by table name; one table stays the plain array |
| Markdown and the documents through it | a table under a heading of its name per table |
| Excel | a worksheet per table; numbers, dates, and booleans as cells of their type |
| standard output | the first table, the rest reported as a loss |

`--sheet` picks a sheet (its every table), a table by its name, or a table
by its number from 1.

## Cells

A cell is written as its stored value, as Excel cells are: numbers in full
(far from 1, in scientific notation: `1.234E-300`), dates in ISO 8601
(`2024-08-08`, or `2024-08-08T14:35:09` with a time, the Excel reader's
forms, so the workbook writer makes them dates again),
booleans as `TRUE` and `FALSE`, durations as Numbers' units (`1w 3d 2h`),
rich text as its lines. Number formats (currency, percentages, thousands
separators, custom date formats) are not applied. A table keeps its full
grid, empty rows and columns included; a merged range keeps its text in
its first cell and leaves the rest empty.

## Packages

- A single-file package (the default): a ZIP of `Index/*.iwa`.
- A package saved as a folder (`Budget.numbers/Index/...`): the command
  line reads the folder as one document, and among other inputs as one
  input.
- A folder package zipped (`Budget.numbers/Index.zip` inside a ZIP): the
  objects are read from the inner `Index.zip`.
- Streams Apple compressed with LZFSE (`bvxn`, the operation log of a
  shared document) hold nothing the reader uses and are kept as bytes.
- Password-protected documents are not read; the error says so.

Only what a cell's text needs is decoded: the formula engine's cell
records, reference tracking, name caches, row and column identity maps,
row sizes, and the tables' layout caches are skipped. Each stream is
decoded and freed in turn, tile rows are kept as their bytes, and a table
is read a tile (256 rows) at a time straight to the writer, never held
whole.

## Checked

`scripts/numbers-check/check.sh` converts numbers-parser's test documents
(https://github.com/masaccio/numbers-parser, MIT) to CSV and compares every
cell with the value numbers-parser reads. On 84 readable documents: 74
match exactly, and 131,713 of 132,206 non-empty cells (99.6%). The rest:

- durations, which we write in Numbers' units and numbers-parser as
  seconds (8 documents);
- `issue-66-collab`, where numbers-parser maps stored rows to table rows
  by the order of the row headers and so moves a block of rows up one; the
  document's preview shows the empty row we keep;
- one rich text cell whose trailing line break we drop.

Speed and memory are the `numbers-csv` and `numbers-xlsx` benchmark pairs
(`DOCS/benchmarks/numbers-csv.md`, `numbers-xlsx.md`); into CSV: 37 MB/s on 200,000-row documents at
125 to 145 MB, against numbers-parser's 0.5 MB/s at over 4 GB, and on a
real 65,000-row document 68 MB/s at 54 MB, against numbers-parser's 2.4 at
353 MB and LibreOffice's 0.9 at 1.4 GB.

Fixtures (`tests/fixtures/numbers`, from numbers-parser's test data): two
sheets with three tables, a zipped package folder with an LZFSE stream, and
an older document whose object headers leave references out (so the
workbook reads every object rather than walking from the root).
