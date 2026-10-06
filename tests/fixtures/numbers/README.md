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
