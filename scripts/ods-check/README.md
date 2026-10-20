# ods-check

Checks ODS conversions against LibreOffice (the reference OpenDocument
implementation), on macOS. None of it runs in CI: it needs LibreOffice
(`soffice`) and a release build (`cargo build --release`). Each script
takes a directory `DIR` with the spreadsheets in `DIR/src/*.ods`.

- `refs.sh DIR`: LibreOffice's CSV export of every sheet, cells as shown,
  into `DIR/ref/<name>-<sheet>.csv` (the reference).
- `o2c.sh DIR`: ODS -> CSV by sublime, scored against the references.
- `c2o.sh DIR`: each reference CSV -> ODS by sublime -> LibreOffice's CSV;
  it must read back as the CSV it came from (the writer's typing rule).
- `o2x.sh DIR`: ODS -> Excel by sublime -> LibreOffice's CSV, against the
  ODS's own references (number formats into Excel's terms).
- `x2o.sh DIR`: LibreOffice saves each ODS as Excel; Excel -> ODS by
  sublime -> LibreOffice's CSV, against LibreOffice's CSV of the workbook
  (Excel's formats into ODS data styles).
- `score.py REF OUT SRC`: byte-identical sheets, matching cells, and
  matching cells outside formulas (LibreOffice recalculates some formulas
  on loading, which no reader controls). `diff.py REF OURS` lists the
  cells that differ.
- `fixtures.py OUT`: writes `tests/fixtures/ods`, built by hand from the
  specification; the CSVs beside them are LibreOffice's export.

The sets used (`DOCS/formats/ods.md`): development, the 297 `.ods` files
of LibreOffice's own tests (`sc/qa/unit/data/ods` of LibreOffice core);
held out, 35 spreadsheets sampled from UK government statistics pages
before any was opened.
