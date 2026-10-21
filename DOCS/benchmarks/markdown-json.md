# Markdown <-> events as JSON

**Latest** (2026-10-21, 1.0.0 release run on the Apple M1 Max: markdown -> markdown-json 61.1 MB/s on markup-dense (73.8 MB) and 276.1 on prose (56.6 MB) at 351.8 MB peak; markdown-json -> markdown 159.1 MB/s at 3.0 MB peak; markdown-json -> markdown throughput FAILS (159.1 against 235.4), every other line PASSES against pulldown-cmark with serde)

## Purpose

`markdown-json` is the event stream serialized, which makes it two things:
a tooling format (inspect or transform a document's structure with `jq`),
and the proof that the Markdown writer round-trips. This pair therefore
also measures the writer, which is what turns Markdown into a middle node
for every format that reaches the events.

## Reference

`pulldown-cmark` events serialized per event by `serde_json`, and deserialized
back and rendered by `pulldown-cmark-to-cmark`. That is the conventional way to
turn a Markdown event stream into JSON and back in Rust, so it is the fair peer
for both directions. Implemented in `bench/src/pairs/markdown_json.rs`.

## Pass lines

| Target | Pass line |
|---|---|
| Markdown -> JSON throughput | >= `pulldown-cmark` events serialized by `serde_json` |
| JSON -> Markdown throughput | >= `serde_json` deserializing those events, rendered by `pulldown-cmark-to-cmark` |
| Peak resident memory, both directions | <= the reference on the same input |

The two sides write their own schemas, so throughput is measured over each
side's own JSON. The sizes are within two percent of each other.

## Method

**Machine, inputs, statistics, and reference options** are those of the
`markdown-html` pair.

**Commands.** `bench/run.sh markdown-json` runs, forward:

- ours: `sublime -q convert big.md out.json --to markdown-json` (and `prose.md`)
- reference: `sublime-bench markdown-json crates big.md out.json`

and back, on the dense input:

- ours: `sublime -q convert out1.json out.md --from markdown-json --to markdown`
- reference: `sublime-bench markdown-json crates-back out2.json out.md`

**Reference pipeline.** `pulldown-cmark` 0.13 with the `serde` feature and
the same extension set as the `markdown-html` pair; `serde_json::to_writer`
per event inside a hand-written array; `serde_json::from_slice` into a
`Vec<Event>` on the way back (an array must be deserialized whole), then
`pulldown-cmark-to-cmark` 22.

**Correctness.** Our JSON is read back by our own reader in CI: every
CommonMark, GFM, and cmark-gfm example round-trips Markdown -> events ->
Markdown -> events with equal events (`tests/markdown_spec.rs`), and the
JSON reader-writer pair has its own round-trip test.

## Threats to validity

- The reference holds the whole event vector in memory on the way back;
  a streaming deserializer would use less memory but is not how the crates
  are normally used.
- The footnote caveat of the `markdown-html` pair applies to the dense
  input.

## Results

### 2026-10-21, 1.0.0 release run (`v1-wrapup` branch)

commit: ec68950 (on the `v1-wrapup` branch, before the 1.0.0 release)
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> markdown-json, markup-dense (73.8 MB): throughput (MB/s of input) | 61.1 | 5.1 | PASS |
| markdown -> markdown-json, prose (56.6 MB): throughput (MB/s of input) | 276.1 | 210.8 | PASS |
| markdown -> markdown-json, markup-dense: peak memory (MB) | 351.8 | 680.3 (reference) | PASS |
| markdown-json -> markdown, markup-dense (317.2 MB): throughput (MB/s of input, each side's own JSON) | 159.1 | 235.4 | FAIL |
| markdown-json -> markdown, markup-dense: peak memory (MB) | 3.0 | 1545.8 (reference) | PASS |

The first attempt at this run read stale, smaller inputs left in
`bench/data` (2.2 MB dense, 1.7 MB prose: the scripts generate only when
the file is missing, and several pairs share the names); they were
deleted, regenerated at the documented sizes, and the pair rerun. The
machine was in use during the rerun, more heavily than during the rest
of this run (load average 16 to 26 on 10 cores: a browser, a VPN
client, a remote-desktop host), so the wall-clock rows are noisier than
earlier blocks'. The machine is the Apple M1 Max, not the Linux one of
the block below, and the prose input is the full 56.6 MB rather than
that block's 4.7 MB, so the rows do not compare with it directly.

markdown-json -> markdown FAILS, where it passed in the block below
(310.5 against 239.4): 159.1 against 235.4 MB/s now. The harness gates
the row on wall time (`bench/pairs/markdown-json.sh`: ours in seconds
<= the reference's), and here our rate is lower as well. A paired A/B
of our binary at ec68950 against the commit before it (93a37d4) gave
the same median on both, but it was run on the stale 2.2 MB input, so
it does not apply at this input size; the cause of this failure is
unmeasured.

### 2026-09-25, 0.17.0 Markdown speedups

commit: 0b33385 (the merge of the rows-to-document bridge, before the version bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| markdown -> markdown-json, markup-dense (73.8 MB): throughput (MB/s of input) | 100.3 | 4.0 | PASS |
| markdown -> markdown-json, prose (4.7 MB): throughput (MB/s of input) | 225.1 | 161.6 | PASS |
| markdown -> markdown-json, markup-dense: peak memory (MB) | 305.2 | 565.2 (reference) | PASS |
| markdown-json -> markdown, markup-dense (317.2 MB): throughput (MB/s of input, each side's own JSON) | 310.5 | 239.4 | PASS |
| markdown-json -> markdown, markup-dense: peak memory (MB) | 2.8 | 1226.6 (reference) | PASS |

Recorded at the 0.17.0 release because the Markdown writer (text
copied in runs, digits scanned only where a list could open), the block
parser (one cell vector per table), and inline rendering (a plain-text
fast path) changed for the rows-to-document bridge. The 4.7 MB prose
inputs run in about fifteen milliseconds, so their throughput medians
swing between runs; the dense rows are the stable ones.

2026-09-23, 0.3.0. Medians of three:

| Target | Ours | Reference | Result |
|---|---|---|---|
| Markdown -> JSON throughput, markup-dense, 86 MB (MB/s) | 98.3 | 4.1 | PASS |
| Markdown -> JSON throughput, prose, 59 MB (MB/s) | 273.2 | 250.8 | PASS |
| Peak RSS Markdown -> JSON, markup-dense (MB) | 302.9 | 562.1 | PASS |
| JSON -> Markdown throughput, markup-dense (MB/s of each side's JSON) | 325.6 | 245.9 | PASS |
| Peak RSS JSON -> Markdown, markup-dense (MB) | 2.4 | 1,224.3 | PASS |

Output of the JSON -> Markdown path re-parses to the events it came from
for every specification example.

## Conclusions

- Forward, on prose, we are 9% ahead of `serde_json` over `pulldown-cmark`
  events with output of the same size. Two things paid for it: the JSON
  writer takes prepared keys and constant values, so nothing is rescanned
  per event, and the first draft's `{"type":...,"tag":...}` schema was
  30% larger than the reference's and cost exactly that in time; the
  key-named schema fixed both size and speed at once.
- Back, we stream: the reader hands each event to the Markdown writer
  borrowing its own buffers, so memory stays at 2.4 MB against 1.2 GB for
  deserializing the array whole, and throughput is a third higher.
  2026-10-21: in the 1.0.0 release run on the Apple M1 Max the back
  direction is slower than the reference (159.1 against 235.4 MB/s) and
  FAILS; memory still streams (3.0 MB against 1545.8).
- The JSON tokenizer now copies plain string runs a word at a time. That
  is shared code: JSON -> CSV went from 171 to 191 MB/s in the same run.
