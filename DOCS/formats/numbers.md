# Apple Numbers (numbers)

A Numbers document is an iWork package like Pages: a ZIP of IWA streams
(Snappy-framed protobuf objects). It is read through the Pages package
reader (`src/io/pages/package.rs`) and the Pages table reader, which shares
the table archives (TST) with Numbers. `io::pages::WorkbookReader` walks the
document's sheets (`TN.DocumentArchive`, `TN.SheetArchive`, read by field
number because the Pages registry leaves Numbers' own types out), each
sheet's drawables to its tables (`TST.TableInfoArchive`, through groups),
and each table's cells, a tile at a time. `io::pages::write_numbers_to`
writes tables back as a Numbers document.

## Status

- `numbers -> csv`, `tsv`, `json`, `markdown`, `xlsx`: shipped, and every
  format those reach (JSON Lines, YAML, TOML, XML, Word, Pages, HTML, RTF,
  PDF).
- `csv`, `tsv`, `xlsx`, `json`, `markdown` `-> numbers`: shipped, and every
  document format through Markdown (a document's tables).

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
| Excel | a worksheet per table; numbers, dates, durations, and booleans as cells of their type, with the Excel format that shows them as Numbers does |
| standard output | the first table, the rest reported as a loss |

`--sheet` picks a sheet (its every table), a table by its name, or a table
by its number from 1.

## Cells as Numbers shows them

Text outputs (CSV, TSV, JSON Lines, JSON, Markdown and the documents
through it) carry each cell as Numbers shows it, through its format
(`src/io/pages/format.rs`): a table's format table, by kind (number,
currency, date, duration, text, boolean), and the document's custom
formats with their conditions.

| Format | Shown as |
|---|---|
| decimal, currency, percent | places (or as many as the number needs, at fifteen significant digits), thousands separators, the currency's symbol (`£1,877`), negatives with a minus, in red (no sign), or in parentheses |
| scientific, fraction, base | `1.23456E+03`; `2 1/3` to the stated accuracy or denominator; any base from 2 to 36, two's complement for negatives in 2, 8, and 16 |
| date | the format's pattern (`EEE, d MMM yyyy`, `'Day #'DDD`), with week fields; the system's short time shows AM and PM after a narrow no-break space, as macOS does |
| duration | Numbers' units and styles (`1w 1d 1h`, `1 week`, `0:01:05`), automatic units included |
| checkbox, rating | `TRUE` or `FALSE`, and the number of stars, as Numbers' own CSV export writes them |
| custom number | Numbers' rules for its custom formats: integer digits padded with zeros or spaces, optional decimals trimmed, the width unused decimals leave turned into leading zeros, conditions, currency symbols, quoted text |
| custom text, custom date | the cell's text in its string; the date in its pattern |
| none | the stored value: numbers at fifteen significant digits, dates in ISO 8601 |

Numbers formats a stored decimal through a double (`123456789012345.12345`
shows as `...345.12`), and ties round to even; both are kept.

Excel output keeps each value's type and gives it the Excel number format
that shows it the same (`"£"#,##0`, `0.0%`, `# ?/?`, `ddd, d mmm yyyy`,
`[h]:mm:ss`, a custom format's `?` placeholders for Numbers' space
padding). What Excel cannot express is shown its nearest way, its value
kept: day of year and week fields (a date in ISO 8601), base formats and
custom formats with conditions (text), the zeros Numbers adds from unused
decimals, and accounting style's tab.

Not shown as Numbers does: a categorised table is written as its rows (its
data), not grouped under category rows as Numbers' view and export show it;
a pivot table's computed cells are not read; formulas are their last value
(Numbers recalculates some, `NOW()` and locale functions, when it opens a
document).

## Writing

`write_numbers_to` starts from a blank document Numbers saved
(`src/io/pages/numbers_template.numbers`, its previews removed, its entries
deflated): one sheet with one table. Further tables are clones of the
table's cluster (model, tiles, data lists, header buckets, and formula
engine owners, registered with the engine and the package metadata) as the
Pages writer clones tables; further sheets are clones of the sheet with its
header, footer, and guide storages (its references are raw bytes, mapped by
hand). The document's sheet list and the sidebar tree (a node per sheet,
under it a node per table) are rebuilt.

| Source | Becomes |
|---|---|
| CSV, TSV | one sheet, one table |
| Excel | a sheet per worksheet, one table each |
| JSON | a sheet per table (an object of arrays a sheet per member) |
| Markdown, and Word, Pages, HTML, RTF, PDF through it | one sheet holding the document's tables, each named after the heading before it |

Cells are typed by their text under the workbook writer's read-back rule
(`xlsx.md`): plain decimals (no exponent: `1e3` would read back as `1000`)
are decimal128 numbers; ISO 8601 dates and dates with times are dates with
a `yyyy-MM-dd` or `yyyy-MM-dd'T'HH:mm:ss` format, so they show as written;
`TRUE` and `FALSE` are booleans; everything else is text. The first row is
the header row; the table has Numbers' default style, no header column, and
default column widths. Formats, merges, and styling are not written yet.

The table layout follows Numbers': a tile per 256 rows, listed in the row
tile tree with the next row strip id after them; no record for an empty
cell and no row record for an empty row; a row's offset array a slot per
column; a table past 255 columns in wide offsets, its tiles marked
`should_use_wide_rows`, its legacy fields holding Numbers' placeholder; a
table past 65,535 rows without the multiple-choice list Numbers drops at
that size. The same tiles serve the Pages writer's tables.

A large table's repeated records (tile rows, strings, row headers, row
identifiers) are encoded as they are built rather than kept field by field,
and its cells are kept as text or a typed value only: 300,000 rows of four
columns write in 0.6 s at about 350 MB peak (933 MB before), to a 23 MB
document.

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

Only what a cell needs is decoded: the formula engine's cell records,
reference tracking, name caches, row and column identity maps, row sizes,
and the tables' layout caches are skipped. Each stream is decoded and freed
in turn, tile rows are kept as their bytes, and a table is read a tile (256
rows) at a time straight to the writer, never held whole.

## Checked

Against Numbers itself (`scripts/numbers-check`, macOS with Numbers and
LibreOffice), on numbers-parser's test documents
(<https://github.com/masaccio/numbers-parser>, MIT):

- `check.sh` exports each document to CSV with Numbers and compares every
  cell we read. As text: 69 of 79 documents exact, 130,008 of 131,593
  non-empty cells (98.8%). Through Excel (our workbook shown by
  LibreOffice): 63 to 64 of 79 exact, 98.7% of cells. The rest are the
  categorised and pivot views, formulas Numbers recalculates on opening,
  macOS's narrow space before AM and PM in one document's built-in time
  format, and, through Excel, what Excel formats cannot express (above).
- `write.sh` rewrites each document through our writer (Numbers, Excel,
  Numbers), opens it in Numbers, and reads Numbers' log and export: all 79
  open with no repair, upgrade, or assertion and export what our reader
  reads; no crashes. So do tables of 65,534, 65,537, and 300,000 rows.

Speed and memory are the `numbers-csv` and `numbers-xlsx` benchmark pairs
(`DOCS/benchmarks/numbers-csv.md`, `numbers-xlsx.md`).

Fixtures (`tests/fixtures/numbers`, from numbers-parser's test data): two
sheets with three tables, a zipped package folder with an LZFSE stream, an
older document whose object headers leave references out, and two
documents of formats with Numbers' own CSV export of each
(`reference/`) as the expected text.
