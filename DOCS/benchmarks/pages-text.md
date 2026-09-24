# Pages -> plain text

**Latest** (2026-09-24: pages -> text 15.9 MB/s of input on the dense shape and 19.3 on prose, at 142.3 and 64.5 MB peak; both goals (50 MB/s, 64 MB) FAIL, see Conclusions and `STATE.md`)

## Purpose

A Pages document as plain text through the Markdown event projection and
the text writer: the "just give me the text" ask, and the least work any
document path can do. If this path cannot meet the goals, no document path
can.

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

**Commands.** `bench/run.sh pages-text` runs, per shape:

- ours: `sublime -q convert styled.pages styled.txt --to text`

The harness module (`bench/src/pairs/pages_text.rs`) exposes `ours` for
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
- The text writer is a few milliseconds; this pair is the clearest
  measure of the document model's own cost.

## Results

### 2026-09-24

commit: 8dbe707
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| pages -> text, styled (2.5 MB): throughput (MB/s of input) | 15.9 | goal: 50 | FAIL |
| pages -> text, styled: peak memory (MB) | 142.3 | goal: <= 64.0 | FAIL |
| pages -> text, prose (1.5 MB): throughput (MB/s of input) | 19.3 | goal: 50 | FAIL |
| pages -> text, prose: peak memory (MB) | 64.5 | goal: <= 64.0 | FAIL |

## Conclusions

- Both goals fail on both shapes at the same ratios as the Markdown and
  HTML pairs. With the writer near zero, the numbers are the reader's:
  3.4 MB of text out of a 2.5 MB package in 0.16 s and 142 MB. The levers
  are the reader's (`pages-docx.md`).
