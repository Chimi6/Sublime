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
| Startup to first byte of output on a 1 KB file | < 1 ms |

## Results

### 2026-09-22

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
