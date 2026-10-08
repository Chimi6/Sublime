# Numbers fixtures

From the test data of numbers-parser (https://github.com/masaccio/numbers-parser),
MIT License, copyright Jon Connell. Preview and preset images were removed to keep
them small; the documents are otherwise unchanged.

- `sheets.numbers` (`test-1.numbers`): two sheets, the first with two
  tables, cells left empty here and there.
- `bundle.numbers` (`issue-32.numbers`): a package saved as a folder and
  zipped, its objects in an inner `Index.zip`, one of its streams LZFSE
  compressed.
- `old.numbers` (`issue-17.numbers`): an older document whose object
  headers leave references out, so a walk from the root misses its cells.
- `formats.numbers` (`test-custom-formats.numbers`) and `currencies.numbers`
  (`test-8.numbers`): dates, fractions, custom number formats, currencies,
  and percentages; `reference/` holds Numbers' own CSV export of each (made
  by Numbers 12.0 on macOS), the text our reader must produce.
- `pivot.numbers` (`test-pivot.numbers`): a pivot table of a source table,
  rows and columns sorted by group with grand totals; `reference/pivot/`
  holds Numbers' CSV export of both tables.
- `categories.numbers` (`test-categories.numbers`): categorised tables,
  grouped by text, number, boolean, and dates (year, quarter, week,
  month, day, weekday), nested to five levels, some columns hidden;
  `reference/categories/` holds Numbers' CSV export of six of them.
