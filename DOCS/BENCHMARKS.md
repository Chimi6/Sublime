# Benchmarks

Results from `bench/run.sh`. Each block records the commit hash, machine, and
the numbers. Pass lines are in the design spec section 13 and repeated here.

## Pass lines

| Target | Pass line |
|---|---|
| CSV -> JSON throughput | >= `csv` crate + `serde_json` streaming pipeline |
| JSON -> CSV throughput (file input) | >= `serde_json::StreamDeserializer` + `csv` writer pipeline |
| Peak RSS at 1 GB input, file source, both directions | < 16 MB |
| Peak RSS at 1 GB input, stdin source, CSV -> JSON | < 16 MB |
| Release binary size, Linux x86_64 musl | < 1 MB |
| Startup to first byte of output on a 1 KB file, above the process-spawn floor | < 1 ms |

## Results

Startup is timed inside the bench binary with `Instant`, from `spawn` to the
first byte of stdout, median of 25 runs. The process-spawn floor is the same
measurement for `/bin/true`. Shell-based timing forks extra processes per
sample and cannot resolve sub-millisecond startups; the first entry below was
measured that way and its startup row is not comparable.

### 2026-09-22, after the first performance spike

10,000,000 rows, 1,183,138,107 bytes CSV. Reference is the `csv` crate reading
records into a streaming `serde_json` map serializer, and `serde_json` into the
`csv` writer, from `bench/src/main.rs`.

| Target | Ours | Reference | Result |
|---|---|---|---|
| CSV -> JSON throughput (MB/s) | 354.7 | 302.2 | PASS |
| JSON -> CSV throughput (MB/s) | 171.3 | 105.5 | PASS |
| Peak RSS CSV -> JSON file (MB) | 2.2 | < 16 | PASS |
| Peak RSS JSON -> CSV file (MB) | 2.0 | < 16 | PASS |
| Peak RSS CSV -> JSON stdin (MB) | 3.4 | < 16 | PASS |
| Binary size, musl static (bytes) | 656000 | < 1048576 | PASS |
| Startup above spawn floor, glibc build (ms) | 0.2 to 0.5 (spawn 0.44 to 0.93, floor 0.28 to 1.45) | < 1 | PASS |
| Startup above spawn floor, musl build (ms) | 0.2 to 0.5 (spawn 0.19 to 0.94, floor 0.28 to 1.45) | < 1 | PASS |

Reader alone on a 122 MB file: ours 0.14 s, `csv` crate 0.10 s. The remaining
gap is in the reader's per-delimiter state machine; the end-to-end win comes
from the buffered writer and the word-at-a-time scanner.

Startup medians were bimodal across repeats (the machine's CPU power state),
and so was the floor, which is why a range is recorded rather than one number.

commit: c0fa6d0
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

### 2026-09-22, before the spike (shell-timed startup, not comparable)

| Target | Ours | Reference | Result |
|---|---|---|---|
| CSV -> JSON throughput (MB/s) | 165.7 | 316.0 | FAIL |
| JSON -> CSV throughput (MB/s) | 172.6 | 103.7 | PASS |
| Peak RSS CSV -> JSON file (MB) | 2.0 | < 16 | PASS |
| Peak RSS JSON -> CSV file (MB) | 2.1 | < 16 | PASS |
| Peak RSS CSV -> JSON stdin (MB) | 3.4 | < 16 | PASS |
| Binary size (bytes) | 651904 | < 1048576 | PASS |
| Startup (ms, 1 KB file) | 1.527 | < 1 | FAIL |

commit: 4ceb6e9
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus
