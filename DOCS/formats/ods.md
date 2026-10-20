# OpenDocument spreadsheet (ods)

An ODS is a ZIP of XML parts (ISO/IEC 26300, ODF 1.3) whose first entry is
`mimetype` (`application/vnd.oasis.opendocument.spreadsheet`), stored,
which is how a file is recognised. `content.xml` holds every sheet
(`table:table`) as rows of cells; `styles.xml` and `content.xml`'s
automatic styles hold the cell styles and their data styles (number
formats). Sublime reads every sheet into rows (`src/io/ods/reader.rs`)
and writes rows into a spreadsheet of one or more sheets
(`src/io/ods/writer.rs`); number formats pass through one model that
reads and writes both ODS data styles and Excel format codes
(`src/io/ods/numfmt.rs`). So an ODS reaches CSV, TSV, JSON, JSON Lines,
Markdown, Excel, Numbers, and every document format, and each of them
reaches ODS. Google Sheets is not a file format: it exports XLSX, CSV, or
ODS, which are read here.

## Status

- `ods -> csv`, `ods -> tsv`: shipped, conditional (a file per sheet, or
  the one `--sheet` picks; JSON Lines through them).
- `ods -> json`, `ods -> markdown`: shipped, conditional (every sheet in
  one file, as for a workbook; every document format through Markdown).
- `ods -> xlsx`, `ods -> numbers`: shipped, conditional (values typed,
  number formats and merged cells kept).
- `csv`, `tsv`, `xlsx`, `json`, `markdown`, `numbers -> ods`: shipped
  (Excel and Numbers with their number formats and merged cells).

Measured with `scripts/ods-check` against LibreOffice, the reference ODS
implementation (its CSV export of every sheet, cells as shown):

| Set | Sheets byte-identical | Cells matching | Cells matching, no formula |
|---|---|---|---|
| ODS -> CSV, 297 development files (LibreOffice's test suite, 463 sheets) | 303 | 98.94% | 99.65% |
| ODS -> CSV, 35 held-out files (UK government statistics, 240 sheets, 12.2 million cells) | 190 | 99.997% | 99.999% |
| CSV -> ODS -> LibreOffice, the 463 development CSVs | 463 | 100% | |
| ODS -> Excel -> LibreOffice, 463 sheets | 387 | 98.89% | 99.33% |
| Excel -> ODS -> LibreOffice, LibreOffice's Excel files of the 297 | 408 | 91.61% | 99.30% |

The held-out files were sampled (every third link of 104 on seven
publication pages) before any of them was opened. They found five faults,
fixed before the numbers above, so the held-out figures are no longer
blind: a second `value()=0` condition LibreOffice reads as `< 0`; text
formats on strings (`_-@_-` pads with spaces); a value repeated across
columns whose default styles differ; text formats left off multi-line
cells; and the reader's speed and memory on a 744 MB `content.xml`.

Excel's paths read lower with formulas because LibreOffice recalculates a
workbook it reopens (external references, pivot tables); the Excel reader
gives times to the second. What remains different is almost all
LibreOffice's, not the file's:
formula cells it recalculates on loading (volatile functions such as
`TODAY` and `RAND`, matrix results, references it re-resolves; the
formula columns above count them), and text inside a styled span within
a link, which LibreOffice drops from a cell and is kept here.

Oracles (`tests/ods_rows.rs`, fixtures in `tests/fixtures/ods` built by
`scripts/ods-check/fixtures.py` from the specification, with LibreOffice's
export of every sheet beside them): every fixture sheet reads to
LibreOffice's CSV byte for byte (strings with spaces, tabs, line breaks,
and links; General, grouped, percent, currency with a red negative
section, scientific, fraction, ISO, locale-order, and long dates,
durations past a day, a date and time a hundredth of a second short of
the minute, conditional `[low]` text, a German-language format, `?`
digits, a text format, booleans stored as text and as numbers, merged and
covered cells, repeated rows and cells, a column's default style, rows
repeated to LibreOffice's limit); sheets are parts by name and number, a
missing one is refused with the list; every CSV fixture survives CSV to
ODS to CSV; near misses stay text (`007`, `1.50`, `true`) while numbers,
exponents, dates, times, and booleans are typed; formats and merges
survive ODS to Excel to ODS; Markdown and JSON become a sheet per table;
Numbers and ODS reach each other.

## Shown text

Text outputs show each cell as LibreOffice shows it to an English reader,
which is also the form its CSV export writes on this machine; Excel and
Numbers get the value with its format.

| Cell | Shown as |
|---|---|
| a string | its paragraphs joined by line breaks, spaces as written, annotations and drawings left out; through its text format when it has one (`_-@_-`) |
| a number, date, time, boolean | its value under its data style, in English (`1234.5` with `#,##0.00` is `1,234.50`; a German document's `1.234,50` too) |
| General | the file's own text with its decimal comma made a point: LibreOffice shows General to the column's width (`17.6` for 17.6028...), and the file stores what it showed |
| a format naming its own language (`number:language`) | the file's own text, which is in that language, as LibreOffice shows it |
| a document language's built-in date (German `DD.MM.YY`) | English's built-in date of the same kind (`MM/DD/YY`), from LibreOffice's locale data (`src/io/ods/builtin.rs`, 44 languages) |
| a formula's error | LibreOffice's English name (`#WERT!` is `#VALUE!`, `Chyba:502` is `Err:502`) |
| a formula | its last value; formulas are not recalculated |

Times are cut to the smallest unit shown, as LibreOffice cuts them
(18:04:59.99 is 6:04 PM). A sheet's extent is its last row and column
with content, capped at LibreOffice's 1,048,576 rows and 16,384 columns;
every row is padded to its width.

## Reading

| Part | Read |
|---|---|
| `META-INF/manifest.xml` | encryption: a password-protected spreadsheet is refused |
| `styles.xml`, `content.xml` before its body | data styles (number, percentage, currency, date, time, boolean, text) with their maps; cell styles and their parents; the document's language |
| `content.xml` body | inflated four megabytes at a time and parsed once; each sheet wanted is gathered compactly (its cells' text, and their values and formats for Excel and Numbers) and handed out whole; sheets not wanted are passed over unparsed |

## Writing

`mimetype` stored first, the manifest, `meta.xml`, `content.xml`
streaming as rows arrive, and `styles.xml` written last with the number
formats cells named (common cell styles over data styles), so no row
waits for a style. A cell is typed only when reading it back gives the
same text, as for Excel:

| Cell text | Written as |
|---|---|
| a plain decimal, shown so by General | a float |
| a decimal with an exponent (`2.59E+005`) | a float with the scientific format that shows it so; text when none does |
| an ISO 8601 date, date and time, or time | a date or duration with a format showing the same form |
| `TRUE` or `FALSE` | a boolean |
| anything else | a string: paragraphs per line, `text:s` for runs of spaces, `text:tab` |

From Excel and Numbers, values keep their types and number formats
(Excel codes become data styles, a `[$-407]` language tag becomes the
style's language) and merged ranges become spanned and covered cells.
Sheet names are cleaned to what LibreOffice accepts (none of
`[ ] * ? : / \`) and made unique. `--effort` applies to the compression.

## Not carried

Fonts, fills, borders, column widths, row heights, conditional
formatting, validation, comments, charts, pictures, and formulas (their
last values are). Numbers' date formats ODS cannot state (week of the
month, day of the year) are shown in ISO 8601, as in Numbers -> Excel.
LibreOffice writes an empty sheet as one empty line; here it is no rows,
as for a workbook.

## Performance

`DOCS/benchmarks/ods-csv.md`.
