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

(none yet)
