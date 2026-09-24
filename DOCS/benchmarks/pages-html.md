# Pages -> HTML

**Latest** (2026-09-24, slim model: pages -> html 26.0 MB/s of input on the dense shape and 24.6 on prose, at 74.8 and 45.0 MB peak; the prose memory goal passes; the rest FAIL, see Conclusions and `STATE.md`)

## Purpose

A Pages document as HTML through the Markdown event projection and the
HTML writer: the browser view of a document, and the last step before PDF.

## Pass lines

The goals every Pages pair shares, defined and justified in
`pages-json.md`: throughput >= 50 MB/s of input and peak memory <= 64 MB on
the benchmark inputs. No peer implementation exists to measure against.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** No large real Pages document can be committed and Pages
cannot be scripted from this machine, so the harness scales fixtures
(`bench/target/release/sublime-bench pages-json gen <seed> <units> <out>`):
a fixture's body text is repeated `units` times with a paragraph break
between copies, every character-indexed attribute table (paragraph and
character styles, lists and levels, list starts, sections, layouts) is
repeated and shifted to match, and the result is written back as a real
package through the lossless JSON form. The stylesheet and every other
stream stay exactly as Pages wrote them. Two shapes:

- `styled`: `text-styles.pages` x 5,000: 2.5 MB package, 7.5 MB of
  decompressed streams, 55,000 paragraphs, 170,000 formatted runs. Short
  paragraphs with a character style change every few words.
- `prose`: `paragraphs.pages` x 2,000: 1.5 MB package, 6.6 MB decompressed,
  30,000 paragraphs, 34,000 runs, with line and page breaks. What most
  documents look like.

**Statistics.** `bench/run.sh <pair>`: three runs per command, median wall
clock of the whole process, peak resident memory from GNU `time`,
throughput in MB/s over the input file's bytes on disk. Rows and units
follow `DOCS/benchmarks/README.md`.

**Commands.** `bench/run.sh pages-html` runs, per shape:

- ours: `sublime -q convert styled.pages styled.html --to html`

The harness module (`bench/src/pairs/pages_html.rs`) exposes `ours` for
in-process runs; inputs come from the `pages-json` pair's generator.

## Threats to validity

- No other tool reads the modern Pages format, so the goals are ours, not
  a peer's; they say whether the path is fast enough to feel free, not
  whether it is the fastest possible.
- Repeated text compresses unusually well: the package is 2.5 MB for 7.5 MB
  of streams where a real resume is 328 KB for 75 KB. MB/s of the package
  is therefore lower than a real document would show; ratios between our
  own paths are trustworthy.
- The scaled document reuses the seed's forty style objects 5,000 times; a
  document with thousands of distinct styles would stress the stylesheet
  reader instead.
- Single machine, sometimes busy; medians of three absorb most of it.
- The HTML writer is a small part of the cost; this pair measures the
  document model and the package decode more than the writer.

## Results

### 2026-09-24, slim document model

commit: b8c1a66
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

Phase 1 of the performance plan: the document model became a text arena with interned properties, links, revisions, and strings (a run is 64 bytes and owns no heap). Same inputs as the block below.

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 26.0 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 74.8 | goal: <= 64.0 | FAIL |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 24.6 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 45.0 | goal: <= 64.0 | PASS |

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> html, styled (2.5 MB): throughput (MB/s of input) | 15.2 | goal: 50 | FAIL |
| pages -> html, styled: peak memory (MB) | 142.3 | goal: <= 64.0 | FAIL |
| pages -> html, prose (1.5 MB): throughput (MB/s of input) | 18.7 | goal: 50 | FAIL |
| pages -> html, prose: peak memory (MB) | 64.3 | goal: <= 64.0 | FAIL |

## Conclusions

- The same picture as `pages-markdown.md`: both goals fail on both shapes,
  at a third of the throughput goal and 2.2x the memory goal on the dense
  shape, with the writer a small part of the cost. The levers are the
  reader's (`pages-docx.md`).
