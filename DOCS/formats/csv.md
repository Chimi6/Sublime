# CSV and TSV (csv, tsv)

Delimited rows, read and written a record at a time in constant memory
(`src/io/csv`): RFC 4180 quoting (a field with the delimiter, a quote, or
a line break is quoted, a quote doubled), any line ending read, `\n`
written, a UTF-8 byte order mark skipped. TSV is the same with a tab.
Every row format reaches every other (JSON, JSON Lines, Excel, Numbers,
Markdown tables and the documents through them).

## Delimiters

CSV is not always comma-separated: spreadsheets in much of Europe export
semicolons (their decimals are written with commas), and some tools write
pipes.

- **Reading.** A CSV input's delimiter is read from its first records
  (up to ten, from its first 64 KB): the delimiter (comma, semicolon,
  tab, or pipe) that every record holds the same number of outside quotes,
  the comma when it does. So a semicolon export reads as semicolons, its
  decimal commas inside fields, and a file no delimiter fits (one column)
  reads as commas. `--delimiter` overrides it.
- **Writing.** CSV is written with commas, or the delimiter `--delimiter`
  names (`;`, `|`, `tab`, or any single character but a quote or a line
  break). TSV always uses a tab.
- Rows one conversion hands another inside a path (JSON to Excel goes
  through CSV) keep whatever delimiter they were written with, read back
  the same way.

A file whose first record is a single field with a stray semicolon (and
whose second has none) stays comma-separated: the counts have to agree.
