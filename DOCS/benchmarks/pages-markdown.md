# Pages -> Markdown

**Latest** (2026-09-24: pages -> markdown 15.3 MB/s of input on the dense shape and 18.1 on prose, at 142.0 and 64.8 MB peak; both goals (50 MB/s, 64 MB) FAIL, see Conclusions and `STATE.md`)

## Purpose

A Pages document projected into the Markdown event stream and written
as Markdown: headings, lists, formatting, links, images, tables, footnotes.
It is the path into every Markdown workflow, and the projection also feeds
the HTML and text pairs.

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

**Commands.** `bench/run.sh pages-markdown` runs, per shape:

- ours: `sublime -q convert styled.pages styled.md --to markdown`

The harness module (`bench/src/pairs/pages_markdown.rs`) exposes `ours` for
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
- The Markdown writer is a small part of the cost (22 ms of 170 ms on
  `styled` in-process); this pair measures the document model and the
  package decode more than the writer.

## Results

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> markdown, styled (2.5 MB): throughput (MB/s of input) | 15.3 | goal: 50 | FAIL |
| pages -> markdown, styled: peak memory (MB) | 142.0 | goal: <= 64.0 | FAIL |
| pages -> markdown, prose (1.5 MB): throughput (MB/s of input) | 18.1 | goal: 50 | FAIL |
| pages -> markdown, prose: peak memory (MB) | 64.8 | goal: <= 64.0 | FAIL |

## Conclusions

- Both goals fail on both shapes: a third of the throughput goal, and
  2.2x the memory goal on the dense shape (the prose shape is within a
  megabyte of it). The writer is cheap; the cost is the document model the
  reader builds and the package decode, shared with every document path
  and diagnosed in `pages-docx.md`.
- The reader levers there (slimmer run, decode on lookup) carry this pair
  with them; once the model is fixed the pair should pass with room, since
  it writes 3.4 MB of Markdown where the Word path writes and deflates
  30 MB of XML.
