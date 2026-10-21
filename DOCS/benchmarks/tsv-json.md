# TSV <-> JSON

**Latest** (2026-10-21, 1.0.0 release run: tsv -> json 174.5 MB/s of input at 2.7 MB peak; json -> tsv 124.5 MB/s at 2.8 MB peak; every line PASSES against the csv crate with serde_json, at 1.5 to 2.1 times its throughput)

## Purpose

TSV is the CSV reader and writer with a tab for a separator, so this
pair checks that the separator parameter cost nothing: the numbers
should stand beside `csv-json.md`'s. It is also where the tab scanner
(`scan::find_tsv_delimiter`) is measured.

## Reference

The `csv` crate with `delimiter(b'\t')` reading records into a streaming
`serde_json` map serializer (TSV to JSON), and `serde_json` into the
`csv` crate's writer with a tab delimiter (JSON to TSV), as in
`csv-json.md`. Implemented in `bench/src/pairs/csv_json.rs`.

## Pass lines

Not slower, and no more memory, than the `csv` crate with `serde_json`,
in each direction.

## Method

**Machine.** Recorded with each results block from the harness's
machine line.

**Inputs.** The `csv-json` pair's generated files with the separator
swapped by our own `csv -> tsv` (quoted fields keep their commas):
594.3 MB of TSV for 10,000,000 rows, and the same 1,110.6 MB JSON array
of string-valued objects the CSV pair reads.

**Statistics.** `bench/run.sh tsv-json`: three runs per command, median
wall clock of the whole process, peak resident memory from GNU `time`,
throughput in MB/s over the input file's bytes. Rows and units follow
`README.md`.

**Commands.**

- ours: `sublime -q convert big.tsv out.json` and `sublime -q convert big.json out.tsv`
- reference: `sublime-bench csv-json crates-tsv-json big.tsv out.json` and `crates-json-tsv big.json out.tsv`

## Threats to validity

- The reference JSON-to-TSV pipeline reads the whole array into memory
  (`Vec` of maps) because the `csv` crate has no two-pass reader; its
  memory row says what that costs, not what the crate could do with
  more work. Ours streams in two passes over the file.
- The rows are short and regular; fields with tabs and quotes are rare
  in the input (one quoted field in seven rows).

## Results

### 2026-10-21, `v1-wrapup` branch, the 1.0.0 release run

commit: ec68950
machine: Darwin 24.5.0 arm64, 10 cpus, Apple M1 Max

| Target | Ours | Reference | Result |
|---|---|---|---|
| tsv -> json, 10M rows (594.3 MB): throughput (MB/s of input) | 174.5 | 114.2 (csv + serde_json) | PASS |
| tsv -> json, 10M rows: peak memory (MB) | 2.7 | 4.7 (csv + serde_json) | PASS |
| json -> tsv, 10M rows (1110.6 MB): throughput (MB/s of input) | 124.5 | 59.1 (serde_json + csv) | PASS |
| json -> tsv, 10M rows: peak memory (MB) | 2.8 | 8012.1 (serde_json + csv) | PASS |

Every line passes, at 1.5 and 2.1 times the reference's throughput. This
run is on a different machine from the previous block (an M1 Max, not
the Linux i7-13700K), so its absolute MB/s do not compare with that
block's; the ratios against the reference in the same session do. The
machine was in use during the run (load average 7 to 12 on 10 cores: a
browser, a VPN client, a remote-desktop host), so the wall-clock rows
are noisier than earlier blocks'.

### 2026-09-25, first release of TSV

commit: 2f7e30d
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus, 13th Gen Intel(R) Core(TM) i7-13700K

| Target | Ours | Reference | Result |
|---|---|---|---|
| tsv -> json, 10M rows (594.3 MB): throughput (MB/s of input) | 325.1 | 220.3 (csv + serde_json) | PASS |
| tsv -> json, 10M rows: peak memory (MB) | 2.7 | 4.3 (csv + serde_json) | PASS |
| json -> tsv, 10M rows (1110.6 MB): throughput (MB/s of input) | 191.8 | 95.8 (serde_json + csv) | PASS |
| json -> tsv, 10M rows: peak memory (MB) | 2.6 | 9998.0 (serde_json + csv) | PASS |

## Conclusions

The separator parameter cost nothing: the rows match `csv-json.md`'s
within noise, and the reader's tab scanner is the same word-at-a-time
scan with a different byte. Nothing here is close to a limit.

On 2026-10-21 the rows did not match `csv-json.md`'s: tsv -> json ran at
174.5 MB/s against csv -> json's 94.8, and json -> tsv at 124.5 against
json -> csv's 88.5, on the same loaded machine in separate sessions. The
references fell with them (114.2 here, 37.1 in the csv-json session), so
the two runs saw different load; this run does not test the separator.
