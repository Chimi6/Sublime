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
| date | the format's pattern (`EEE, d MMM yyyy`, `'Day #'DDD`), with week fields; a formula's result under Numbers' automatic format in the system's short time shows AM and PM after a narrow no-break space, as macOS does (a cell's record marks the format kinds it chose, byte 6: 0x01 number, 0x02 currency, 0x04 duration, 0x08 date, 0x80 text) |
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

A pivot table reads as Numbers shows it. Its model stores the body in the
order the groups were made; the table's view (`view_column_row_uids` on
`TST.TableInfoArchive`) orders every row and column by uid, and the
summary model (`TST.SummaryModelArchive`, its own `data_store` and uid
map) holds the grand total row and column. Each shown cell is the
summary's at that row and column if it has one, else the stored cell. The
workbook read keeps these maps and the summary model for pivot tables
alone (found by the references in the objects' headers), and reads a
pivot's two grids whole.

Not shown as Numbers does: a categorised table is written as its rows (its
data), not grouped under category rows with its hidden columns left out, as
Numbers' view and export show it (built from its group-by tree, not a view
map); formulas are their last value
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
| Excel | a sheet per worksheet, one table each, with its number formats and merged cells |
| JSON | a sheet per table (an object of arrays a sheet per member) |
| Markdown, and Word, Pages, HTML, RTF, PDF through it | one sheet holding the document's tables, each named after the heading before it |

Cells are typed by their text under the workbook writer's read-back rule
(`xlsx.md`): plain decimals (no exponent: `1e3` would read back as `1000`)
are decimal128 numbers; ISO 8601 dates and dates with times are dates with
a `yyyy-MM-dd` or `yyyy-MM-dd'T'HH:mm:ss` format, so they show as written;
`TRUE` and `FALSE` are booleans; everything else is text. The first row is
the header row; the table has Numbers' default style, no header column, and
default column widths. Styling is not written.

From Excel, each cell's number format becomes the Numbers format that
shows it the same (`format::from_excel`), written to the table's format
list with the fields Numbers writes for its kind, and named by the cell
record (the format kind it shows at `0x1000`, the key at `0x2000` for a
number, `0x4000` and record kind 10 for a currency, `0x8000` for a date),
and marked as chosen rather than automatic (byte 6, as Numbers marks a
format picked in its inspector):

| Excel | Numbers |
|---|---|
| `0.00`, `#,##0`, red or parenthesised negatives | decimal: places, thousands separator, negative style |
| `"£"#,##0.00`, `[$€-407]#,##0.00`, accounting | currency by its ISO code |
| `0.0%`, `0.00E+00`, `# ?/?`, `# ?/8` | percent, scientific, fraction to digits or a denominator |
| date and time codes (`d mmm yyyy`, `h:mm AM/PM`) | the date pattern in ICU (`d MMM yyyy`, `h:mm a`); a time alone on Excel's day zero |
| General, text, elapsed time (`[h]:mm`), padding (`000`, `??`), optional decimals beside fixed ones, scaling, engineering notation, text around the digits | none: the value as it is |

Under a number format a cell is a number whatever its digits (the
workbook says so). Merged ranges are written as the Pages writer writes
them; the cells they cover are left empty.

The table layout follows Numbers': a tile per 256 rows, listed in the row
tile tree with the next row strip id after them; no record for an empty
cell and no row record for an empty row; a row's offset array a slot per
column; every row in wide offsets (4-byte units, `has_wide_offsets`), its
tile marked `should_use_wide_rows` and its legacy (pre-BNC) fields holding
Numbers' placeholder, as Numbers writes its own large tables (a narrow
row would need a copy of its records there, which Numbers checks); a
table past 65,535 rows without the multiple-choice list Numbers drops at
that size. The same tiles serve the Pages writer's tables (narrow, with
their copy).

Memory: the source rows are held compactly (every cell's text in one
buffer, `NumbersRows`), a large table's repeated records (tile rows,
strings, row headers, row identifiers) are encoded as they are built and
run together into one tree entry each (an entry written verbatim), and the
tiles' bytes are sized once. 300,000 rows of four columns write in 0.6 s at
about 175 MB peak (350 MB before, 933 MB before that) to a 19.8 MB
document; 1,000,000 rows in 2.1 s at about 500 MB (900 MB before) to 70 MB.
Numbers opens tables of up to 1,000,000 rows and 1,000 columns; a larger
table is written with a warning.

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
  cell we read. As text: 72 of 79 documents exact, 130,044 of 131,593
  non-empty cells (98.8%). Through Excel (our workbook shown by
  LibreOffice): 65 of 79 exact, 98.7% of cells. The rest are the
  categorised view, formulas Numbers recalculates on opening,
  and, through Excel, what Excel formats cannot express (above).
- `write.sh` rewrites each document through our writer (Numbers, Excel,
  Numbers), opens it in Numbers, and reads Numbers' log and export: all 79
  open with no repair, upgrade, or assertion and export what our reader
  reads; no crashes. So do tables of 65,534, 65,537, 300,000, 600,000,
  and 1,000,000 rows, Numbers' export of each the source exactly (the
  largest opens in about three minutes).
- Formats and merges from Excel: each test document through our workbook
  and back (Numbers, Excel, Numbers) opens in Numbers cleanly, and
  Numbers' export of it matches LibreOffice's view of the workbook in 67
  of 79 documents and 99.1% of cells; the rest are Excel formats Numbers'
  built-in ones cannot say (padding, optional decimals), left
  unformatted. A workbook of every mapped format and three merges
  (`tests/fixtures/xlsx/formats.xlsx`) shows in Numbers as in Excel.

Speed and memory are the `numbers-csv` and `numbers-xlsx` benchmark pairs
(`DOCS/benchmarks/numbers-csv.md`, `numbers-xlsx.md`).

Fixtures (`tests/fixtures/numbers`, from numbers-parser's test data): two
sheets with three tables, a zipped package folder with an LZFSE stream, an
older document whose object headers leave references out, and two
documents of formats and a pivot table with Numbers' own CSV export of
each (`reference/`) as the expected text.
