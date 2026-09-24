# Benchmark harness

Manual only. Nothing here runs in CI, and the harness is excluded from the
main workspace so its dependencies (`csv`, `serde`, `serde_json`) never touch
the shipped binary.

    bench/run.sh <pair> [rows]         run one pair, print a results table
    bench/run.sh csv-json 200000       small run for fast iteration
    bench/run.sh csv-json              full run, 10,000,000 rows

Small runs validate the scripts and show direction while iterating; their
pass/fail column is not meaningful because process spawn and timer overhead
dominate a file of a few megabytes. Only a full run is recorded.

Generated inputs land in `bench/data/` (gitignored, several GB for a full
run). Delete the directory to regenerate with a different row count.

The `sublime-bench` binary exposes each pair's modes directly:

    bench/target/release/sublime-bench csv-json gen-csv 1000000 data.csv
    bench/target/release/sublime-bench csv-json ours-csv-read data.csv
    bench/target/release/sublime-bench startup target/release/sublime tiny.csv
    bench/target/release/sublime-bench pages-json gen tests/fixtures/pages/text-styles.pages 5000 big.pages

## Layout

    run.sh            shared timing helpers, binary size, startup
    pairs/<pair>.sh   one script per pair, defines run_pair
    src/main.rs       dispatch only
    src/common.rs     helpers every pair uses
    src/startup.rs    spawn-to-first-byte and spawn floor
    src/pairs/        one module per pair

## Adding a pair

1. `src/pairs/<name>.rs` with a `pub fn run(mode, args)`.
2. A line in `src/pairs/mod.rs` (`pub mod`, `NAMES`, one match arm).
3. `pairs/<name>.sh` defining `run_pair`, printing rows with `row`.
4. `DOCS/benchmarks/<name>.md` following the template in that folder's README.

## When to run

On cause, not on schedule: any commit that changes a converter's hot path,
and every release. Record the result in the pair's document with the commit
hash and machine, ideally on the same machine as the previous entry.
