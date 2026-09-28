# Benchmarks

One document per conversion pair, written like a methods section: what was
measured, how, under what constraints, and what the numbers do and do not
show. The harness that produces the numbers is in `bench/`; it is run by hand,
never in CI, and only on cause: a change to a converter's hot path, or a
release.

A pair is unordered (`csv-json` covers both directions) because the two
directions share a generator and each direction's output is the other's
input. Each direction is still its own converter with its own pass line,
reference pipeline, and results.

## Standard rows and units

Every results table has the same shape, so any two documents can be read
against each other: `csv -> json` at 333 MB/s and `pages -> docx` at
10 MB/s are the same measurement of different work.

- **Throughput row**, one per direction and input shape:
  `<from> -> <to>, <shape> (<input size> MB): throughput (MB/s of input)`.
  MB/s is the input file's bytes on disk divided by the median wall clock
  of the whole process, spawn included. Bytes are what the user hands us,
  compressed or not, so a compressed format (Pages) reads low per byte;
  the extra row below says why.
- **Compressed-output paths.** When the output is a compressed package
  (Word, and later EPUB, ODF, and the Office family), the throughput row
  counts the bytes the converter handles: the input plus the output's
  uncompressed bytes, since the compressor's work is proportional to what
  it must compress, not to the input. The label says so:
  `<from> -> <to>, <shape> (<in> MB in + <out> MB out): throughput (MB/s of input plus uncompressed output)`.
  The goal number is the same as for every other path of the format.
- **Compressed-input paths.** When the input is a compressed package
  (Word in), the throughput row counts the input's uncompressed bytes,
  what the reader parses, for the same reason; the rate per file byte is
  an extra row. The label says so:
  `<from> -> <to>, <shape> (<in> MB uncompressed): throughput (MB/s of uncompressed input)`.
- **Peak memory row**, one per direction and shape:
  `<from> -> <to>, <shape>: peak memory (MB)`, the process's maximum
  resident set from GNU `time`.
- **Extra rows** are allowed for work the standard rows hide (decompressed
  bytes, a stdin variant, a floor), marked `[extra]` in the target or
  named in the reference cell; they never replace the standard rows.
- **Units.** MB is 1,048,576 bytes everywhere: sizes, throughput, memory.
  Time is the median of three runs of the whole process. Nothing is
  reported per second of CPU or in-process.
- **Reference cell.** The named reference's number, run in the same session
  on the same input: a peer crate compiled into the bench binary, or an
  external tool (pandoc, lynx) spawned and timed the same way, labelled with
  the tool in parentheses. A pair with two references uses two reference
  columns (`Reference 1`, `Reference 2`); the pass gate is the tighter peer,
  the other is context. When no tool anywhere does the conversion, a stated
  goal (`goal: 50`, `goal: <= 64.0`) that the pair's document defines and
  justifies, the same goal for every direction and shape of the format, so
  the rows read alike. Never a number copied from elsewhere, and never one
  direction of a pair judged against the other.
- **Result cell.** PASS or FAIL against that line; `n/a` for a recorded
  extra row with no line.
- **Latest** line at the top of every document: the standard rows' current
  numbers in one sentence, so the file answers "how fast" without
  scrolling. It is updated with every results block.
- **Binary size and startup** are binary-wide, not per pair; they are
  measured with `bench/run.sh binary` and recorded per release in
  `binary.md`, never in a pair document, where they would go stale with
  the next change.

An owner-supplied input that is not committed (a real photograph, a
real document) may carry rows marked `[stock]`, run only when the file
is present; the pair document says which file and why. They are not
pass lines for anyone else.

## Template

Every pair document has these sections, in this order.

1. **Purpose.** What the pair is and why its performance matters.
2. **Reference.** What we compare against and why it was chosen, in a
   sentence or two. A reference is any real, conventional way to do the same
   job: a Rust crate compiled into the bench binary, or a widely used tool
   in another language (pandoc, lynx, LibreOffice) run as an external
   process and timed the same way. A cross-language or general-purpose tool
   does more work than we do, so it is a loose reference — beating it is
   expected — but a real number from a tool people actually use beats an
   invented one. Prefer a real tool wherever one exists in any language; a
   stated goal is the fallback only when no tool anywhere does the
   conversion (the Pages format, a near-identity conversion). If a plausible
   tool exists but is not yet wired in, name it here so the gap is visible
   rather than silently a goal.
3. **Pass lines.** One row per direction. A pass line is a comparison
   against the reference above on the same machine in the same session,
   never an absolute number copied from elsewhere.
4. **Method.** Machine. How inputs are generated and their shape. The exact
   commands. How many runs and which statistic is reported. How memory,
   size, and startup are measured. What the reference pipelines are, with
   the file that implements them.
5. **Threats to validity.** Everything that could make the numbers mislead:
   synthetic data shape, page cache state, CPU power state, a single
   machine, choices made in the reference implementation.
6. **Results.** Dated blocks, newest first, each with the commit hash and
   machine line printed by the harness. Never edit an old block; add a new
   one.
7. **Conclusions.** What the numbers say to do next, and what they do not
   justify claiming.

## Recording a result

Run `bench/run.sh <pair>`, copy the printed table, commit line, and machine
line into a new results block, and note anything unusual about the run. If a
pass line fails, record it anyway; a failed row in the document is the
blocker entry in `STATE.md` pointing back here.
