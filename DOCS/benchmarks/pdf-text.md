# PDF -> Text

**Latest** (2026-10-21, the 1.0.0 release run: pdf -> text at 36.3 MB/s of PDF on `story`, 2.4 on `gs`, and 16.3 on `base14`, at 10.5, 21.6, and 13.9 MB peak; every throughput line PASSES, and the `gs` and `base14` memory lines FAIL, against pdf-extract's 10.0 MB on `gs` and pdftotext's 13.2 MB on `base14`)

## Purpose

A PDF's text read page by page through the content-stream interpreter,
the font decoders, and the line layout, written as pdftotext lays it
out, against the C tool everyone uses and the Rust crate for it.

## Reference

poppler's `pdftotext` (C, default reading-order mode) and the
`pdf-extract` crate (lopdf underneath). Implemented in
`bench/src/pairs/pdf_text.rs`; pdftotext is run by the pair script.

## Pass lines

Not slower, and no more memory, than either reference, on each input.
Throughput counts the PDF's bytes. The words each tool writes are
recorded as a check that the work was done.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** Generated once (`bench/pairs/pdf-text.sh`, Python with
PyMuPDF, and Ghostscript): `story`, a 300-page report of sections,
paragraphs, and lists set in an embedded TrueType font (Type0,
Identity-H, ToUnicode); `gs`, the same report rewritten by Ghostscript
(CID fonts whose glyph units are twice the usual em); `base14`, 300
pages of 48 lines in Helvetica without embedded widths (a simple
WinAnsi font), each line its own small content stream, 14,400 in all.

**Statistics.** `bench/run.sh pdf-text`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`;
outputs are removed before each run.

## Threats to validity

- Generated documents: real PDFs mix fonts, columns, images, and
  annotations. The fixtures in `tests/fixtures/pdf-text` prove the
  harder layouts; these time the common ones at scale.
- pdf-extract writes 5,978 of the `gs` input's 116,700 words (it does
  not read those CID fonts); its time there is for less work.

## Results

### 2026-10-21, the `v1-wrapup` branch, the 1.0.0 release run

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> text, story (2.9 MB, 300 pages): throughput (MB/s of PDF) | 36.3 | 2.8 (pdf-extract); 13.9 (pdftotext) | PASS |
| pdf -> text, story: peak memory (MB) | 10.5 | 17.6 (pdf-extract); 11.8 (pdftotext) | PASS |
| pdf -> text, story: words out [extra] | 116700 | 116700 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, gs (0.2 MB, 300 pages): throughput (MB/s of PDF) | 2.4 | 1.6 (pdf-extract); 1.4 (pdftotext) | PASS |
| pdf -> text, gs: peak memory (MB) | 21.6 | 10.0 (pdf-extract); 12.1 (pdftotext) | FAIL |
| pdf -> text, gs: words out [extra] | 116700 | 5978 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, base14 (3.1 MB, 300 pages): throughput (MB/s of PDF) | 16.3 | 2.3 (pdf-extract); 8.6 (pdftotext) | PASS |
| pdf -> text, base14: peak memory (MB) | 13.9 | 30.3 (pdf-extract); 13.2 (pdftotext) | FAIL |
| pdf -> text, base14: words out [extra] | 172800 | 172800 (pdf-extract); 172800 (pdftotext) | n/a |

commit: ec68950
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

The machine was in use during this run (load average 7 to 12 on 10
cores: a browser, a VPN client, a remote-desktop host), so the
wall-clock rows are noisier than the earlier blocks'. This block is from
the M1 Max laptop and the earlier ones from the Linux bench machine.

The inputs were regenerated for this run: PyMuPDF and Ghostscript were
no longer on this machine, so `bench/pairs/pdf-text.sh` ran with
PyMuPDF (pymupdf.VersionBind 2.0) in a fresh Python 3.13 venv and
Ghostscript 10.08.0 from Homebrew. The `gs` input is Ghostscript's
rewrite of the report, so a different Ghostscript makes a different
file: the `gs` rows' input changed with the tool version. The sizes,
page counts, and words out match the earlier blocks.

Two lines that passed in the 0.24.0 release block FAIL now, both on
memory. `gs`: 21.6 MB against 10.0 (pdf-extract) and 12.1 (pdftotext),
where it was 5.8 against 8.5 and 17.4. `base14`: 13.9 MB against 13.2
(pdftotext), where it was 9.2 against 25.9 (pdf-extract) and 17.7
(pdftotext). Every throughput line and the `story` memory line still
pass.

### 2026-09-27, 0.24.0 release

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> text, story (2.9 MB, 300 pages): throughput (MB/s of PDF) | 66.8 | 3.9 (pdf-extract); 22.8 (pdftotext) | PASS |
| pdf -> text, story: peak memory (MB) | 7.4 | 14.1 (pdf-extract); 17.2 (pdftotext) | PASS |
| pdf -> text, story: words out [extra] | 116700 | 116700 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, gs (0.2 MB, 300 pages): throughput (MB/s of PDF) | 4.2 | 2.2 (pdf-extract); 2.1 (pdftotext) | PASS |
| pdf -> text, gs: peak memory (MB) | 5.8 | 8.5 (pdf-extract); 17.4 (pdftotext) | PASS |
| pdf -> text, gs: words out [extra] | 116700 | 5978 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, base14 (3.1 MB, 300 pages): throughput (MB/s of PDF) | 23.6 | 2.4 (pdf-extract); 14.0 (pdftotext) | PASS |
| pdf -> text, base14: peak memory (MB) | 9.2 | 25.9 (pdf-extract); 17.7 (pdftotext) | PASS |
| pdf -> text, base14: words out [extra] | 172800 | 172800 (pdf-extract); 172800 (pdftotext) | n/a |

commit: e8088c4 (main at the release, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

### 2026-09-27, pdf-text branch

Before the small-inflate change (PR 103) the `base14` input read at 5.0
MB/s against pdftotext's 14.0: each of its 14,400 content streams paid
a megabyte slab and three table builds. It now reads at 23.7.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pdf -> text, story (2.9 MB, 300 pages): throughput (MB/s of PDF) | 69.5 | 3.9 (pdf-extract); 22.3 (pdftotext) | PASS |
| pdf -> text, story: peak memory (MB) | 7.6 | 14.3 (pdf-extract); 17.5 (pdftotext) | PASS |
| pdf -> text, story: words out [extra] | 116700 | 116700 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, gs (0.2 MB, 300 pages): throughput (MB/s of PDF) | 4.1 | 2.3 (pdf-extract); 2.0 (pdftotext) | PASS |
| pdf -> text, gs: peak memory (MB) | 5.7 | 8.5 (pdf-extract); 17.3 (pdftotext) | PASS |
| pdf -> text, gs: words out [extra] | 116700 | 5978 (pdf-extract); 116700 (pdftotext) | n/a |
| pdf -> text, base14 (3.1 MB, 300 pages): throughput (MB/s of PDF) | 23.4 | 2.5 (pdf-extract); 14.1 (pdftotext) | PASS |
| pdf -> text, base14: peak memory (MB) | 9.3 | 26.3 (pdf-extract); 17.8 (pdftotext) | PASS |
| pdf -> text, base14: words out [extra] | 172800 | 172800 (pdf-extract); 172800 (pdftotext) | n/a |

commit: 3191197
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K
