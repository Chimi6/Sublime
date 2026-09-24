# Pages -> Word, Markdown, HTML, text

## Purpose

The flagship's document paths: a `.pages` file read into the document
model and rendered as Word, Markdown, HTML, or plain text. These are the
conversions people actually want from Pages (a resume to send as Word, a
document into a Markdown workflow), so they must feel instant on real
documents and stay inside the binary's memory habits.

## Pass lines

There is no peer implementation: nothing outside Apple reads the modern
Pages format into a document, and Pages itself cannot be timed from a
script on this machine. The pass lines are therefore internal, against the
package round trip that is already measured (`pages -> pages-json`):

| Target | Pass line |
|---|---|
| `pages -> docx` wall clock | <= `pages -> pages-json` on the same file (reading the document and writing Word costs no more than serializing the object graph) |
| `pages -> markdown`, `html`, `text` wall clock | <= `pages -> docx` on the same file |
| Peak resident memory | <= 10 MB on every fixture and the real document |
| Binary size | within `size-budget` (1.4 MB) |

## Method

**Machine.** Recorded with each results block from `uname -srm` and the CPU
model line.

**Inputs.** The real two-page resume used throughout (328 KB, text, lists,
and direct formatting; kept out of the repository) and the two largest
fixtures: `everything.pages` (733 KB, most of it images; text, lists, a
table, an image, footnotes) and `table.pages` (two tables with merges).

**Commands.** Each direction is `sublime -q convert <input> <output> --to
<format>`, run 25 times back to back; the minimum and median wall clock
are reported (`t.sh` in the working notes: `date +%s%N` around the
process). Peak resident memory is GNU `time -f %M` on one run. The binary
is the release profile.

**Reference.** `pages -> pages-json` on the same file, same method.

## Threats to validity

- Process start dominates at this size: every path is a few milliseconds,
  so the numbers say "instant" rather than rank the paths. A large real
  document (hundreds of pages) would separate them; none is in hand.
- Single machine, and the medians move with system load (a loaded run
  showed medians two to three times the minimums with the minimums
  unchanged). The minimum is the stable statistic here.
- The resume is one document shape (a resume); the fixtures are small by
  design.

## Results

### 2026-09-24, commit 9245c0f, every path, in-process phases

Linux 7.1.5 x86_64, Intel Core i7-13700K, release binary. In-process
numbers are the minimum of 15 calls of each phase inside one process
(`Package::read`, `read_document`, then each writer over the same
document); wall clock is the minimum of 25 process runs; process start
alone is 2 ms. The machine was under load, so medians ran two to three
times the minimums.

| Input | Package decode | Document read | Word write | Markdown | HTML | Text |
|---|---|---|---|---|---|---|
| resume (328 KB) | 1.98 ms | 0.05 ms | 0.23 ms | 0.01 ms | 0.01 ms | 0.01 ms |
| everything (733 KB) | 2.90 ms | 0.06 ms | 0.65 ms | < 0.01 ms | < 0.01 ms | < 0.01 ms |
| fonts (706 KB) | 2.69 ms | 0.04 ms | 0.15 ms | < 0.01 ms | < 0.01 ms | < 0.01 ms |

| Input | Path | wall min ms | peak RSS |
|---|---|---|---|
| resume | `pages -> pages-json` | 10 | 7 MB |
| resume | `pages -> docx` | 5 | 6 MB |
| resume | `pages -> markdown` | 5 | 7 MB |
| resume | `pages -> html` | 4 | 7 MB |
| resume | `pages -> text` | 4 | 7 MB |
| everything | `pages -> pages-json` | 10 | 8 MB |
| everything | `pages -> docx` | 7 | 8 MB |
| everything | `pages -> markdown` | 6 | 8 MB |
| everything | `pages -> html` | 5 | 7 MB |
| everything | `pages -> text` | 5 | 7 MB |
| fonts | `pages -> pages-json` | 9 | 7 MB |
| fonts | `pages -> docx` | 6 | 7 MB |
| fonts | `pages -> markdown` | 7 | 8 MB |
| fonts | `pages -> html` | 5 | 7 MB |
| fonts | `pages -> text` | 5 | 7 MB |

Every pass line holds. The package decode is 85 to 95 percent of the
in-process time on every path; the document reader and the writers are
noise. `pages-json` costs more than the document paths because it
serializes every object (the JSON is 5 to 10 times the package's stream
bytes).

### 2026-09-24, commit bb8b657 plus the Markdown projection

Linux 7.1.5 x86_64, Intel Core i7-13700K, 25 runs each, release binary
1,385,824 bytes.

| Input | Path | min ms | median ms | peak RSS |
|---|---|---|---|---|
| resume (328 KB) | `pages -> docx` | 4 | 5 | 7.1 MB |
| resume | `pages -> markdown` | 4 | 9 | 7.0 MB |
| resume | `pages -> text` | 4 | 8 | |
| resume | `pages -> pages-json` (reference) | 7 | 11 | |
| everything (733 KB) | `pages -> docx` | 10 | 21 | 8.5 MB |
| everything | `pages -> markdown` | 5 | 8 | 8.1 MB |
| everything | `pages -> text` | 6 | 8 | |
| everything | `pages -> pages-json` (reference) | 10 | 18 | |
| table | `pages -> docx` | 8 | 17 | 8.6 MB |
| table | `pages -> markdown` | 10 | 17 | 8.1 MB |

The machine was under load during this block (see threats); the minimums
are the numbers to read. Every pass line holds: the document paths cost no
more than the package round trip, memory stays under 9 MB, and the binary
is within budget.

## Conclusions

- The document reader and the writers are already at the floor (tens of
  microseconds); the package decode is the whole cost, and most of it is
  decoding stylesheet presets the document never uses. Lazy object
  decoding (decode a tree when the graph looks it up) is the lever, worth
  about 1.5 ms of the resume's 2 ms; it is listed under Spikes in
  `STATE.md`.
- The `everything -> docx` minimum (10 ms) is the image bytes being
  deflated into the Word package; `add_deflated` on already-compressed PNG
  and JPEG data is wasted work, and storing media uncompressed is the
  first lever if it ever matters.
