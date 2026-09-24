# Text -> Markdown

**Latest** (2026-09-24, first release of the text reader: text -> markdown 256.0 MB/s of input on the dense shape and 442.1 on prose, at 16.7 and 58.8 MB peak; every line PASSES)

## Purpose

Plain text read as paragraphs (runs of lines between blank lines) and
written as Markdown, with everything that would read as markup escaped.
It measures the text reader and the Markdown writer's escaping; the
same reader feeds `text -> html` and `text -> docx`.

## Pass lines

Throughput >= 50 MB/s of input and peak memory <= 64 MB on the benchmark
inputs: the goals the document paths share (`pages-json.md`). No peer
turns text into Markdown.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** Two shapes, 100,000 units each: `prose`, the `markdown-html`
generator's prose written to plain text by our own writer (56.1 MB of
long lines), and `dense`, the harness's own lines full of characters
Markdown would read as markup (`*`, `_`, `#`, `[`, `<`, backticks,
pipes, backslashes; 13.8 MB), which makes the writer escape the most.

**Statistics.** `bench/run.sh text-markdown`: three runs per command,
median wall clock of the whole process, peak resident memory from GNU
`time`, throughput in MB/s over the input file's bytes. Rows and units
follow `README.md`.

**Commands.** Per shape: `sublime -q convert prose.txt prose.md --to markdown`.

## Threats to validity

The reader is a line splitter; what the pair measures is the Markdown
writer's escaping, which the dense shape exercises harder than any real
text would.

## Results

### 2026-09-24, first release of the text reader

commit: 34a9e90 (the reader's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| text -> markdown, dense (13.8 MB): throughput (MB/s of input) | 256.0 | goal: 50 | PASS |
| text -> markdown, dense: peak memory (MB) | 16.7 | goal: <= 64.0 | PASS |
| text -> markdown, prose (56.1 MB): throughput (MB/s of input) | 442.1 | goal: 50 | PASS |
| text -> markdown, prose: peak memory (MB) | 58.8 | goal: <= 64.0 | PASS |

## Conclusions

Five to nine times the goal. Memory on prose is the input held whole
(56 MB) since the writer borrows every line from it; a streaming
converter would read line by line, which nothing here needs yet.
