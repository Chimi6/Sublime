# Binary size and startup

Binary-wide measurements, recorded once per release with `bench/run.sh
binary` rather than in every pair document, where they would go stale
with the next change. The size line is `size-budget` on the gnu binary
(what CI checks); the musl static binary is the release asset and is
recorded beside it. The WebAssembly module (`wasm/`) has its own line
against `wasm/size-budget`, and its gzipped size is recorded because
that is what a browser downloads. Startup is spawn to first output byte on a 1 KB CSV,
less the spawn floor of `/bin/true`, median of runs from
`bench/src/startup.rs`.

## Pass lines

| Target | Pass line |
|---|---|
| Binary size, gnu | <= `size-budget` |
| WebAssembly module | <= `wasm/size-budget` |
| Startup above spawn floor | < 1 ms |

## Results

### 2026-09-24, 0.10.0 with the HTML and text readers

commit: 34a9e90 (the working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| Binary size, gnu (bytes) | 1592688 | <= 1600000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1696384 | recorded | n/a |
| WebAssembly module (bytes) | 767166 | <= 800000 (wasm/size-budget, what CI checks) | PASS |
| WebAssembly module, gzipped (bytes, what a browser downloads) | 314842 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.015 (spawn 0.497, floor 0.512) | < 1 | PASS |

The HTML reader, the text reader, six converters, and the streaming
Word writer added 49 KB to the binary and 24 KB to the module; both
budgets were raised (changelog).

### 2026-09-24, 0.9.0 with the events bridge

commit: e25c786 (the bridge's working tree, before its commit)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| Binary size, gnu (bytes) | 1544072 | <= 1560000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1643136 | recorded | n/a |
| WebAssembly module (bytes) | 743164 | <= 750000 (wasm/size-budget, what CI checks) | PASS |
| WebAssembly module, gzipped (bytes, what a browser downloads) | 306126 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.214 (spawn 0.535, floor 0.749) | < 1 | PASS |

The bridge, its converter, and the projection's formatting stack added
28 KB to the binary and 15 KB to the module; the binary budget was raised
for it (changelog).

### 2026-09-24, 0.8.0 with the Word reader

commit: 436b708 (the feature commit, before the release bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| Binary size, gnu (bytes) | 1515672 | <= 1530000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1618560 | recorded | n/a |
| WebAssembly module (bytes) | 728388 | <= 750000 (wasm/size-budget, what CI checks) | PASS |
| WebAssembly module, gzipped (bytes, what a browser downloads) | 301208 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.007 (spawn 0.510, floor 0.517) | < 1 | PASS |

The Word reader added 86 KB to the binary and 47 KB to the module; both
budgets were raised for it (changelog).

### 2026-09-24, 0.7.0 with the WebAssembly module

commit: 1084f36 (the commit before the release bump)
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| Binary size, gnu (bytes) | 1420416 | <= 1430000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1520256 | recorded | n/a |
| WebAssembly module (bytes) | 681266 | <= 700000 (wasm/size-budget, what CI checks) | PASS |
| WebAssembly module, gzipped (bytes, what a browser downloads) | 281949 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | 0.226 (spawn 0.508, floor 0.282) | < 1 | PASS |

The module is built with `opt-level = "z"`; `"s"` gave 753,596 bytes for
the same fixture timing within noise (7.8 versus 8.9 ms on
`text-styles.pages` under node).

### 2026-09-24, 0.6.0 plus the Pages benchmark work

commit: c60d037
machine: Linux 7.1.5-ogc5.1.fc44.x86_64 x86_64, 24 cpus

| Target | Ours | Reference | Result |
|---|---|---|---|
| Binary size, gnu (bytes) | 1399665 | <= 1400000 (size-budget, what CI checks) | PASS |
| Binary size, musl static (bytes, the release asset) | 1499776 | recorded | n/a |
| Startup above spawn floor (ms, 1 KB file) | -0.117 (spawn 0.857, floor 0.974) | < 1 | PASS |

The startup row moves by a millisecond either way with machine load and is
read as "under a millisecond", not as a trend.

## History

| Release | gnu bytes | musl bytes | budget | wasm bytes | wasm budget |
|---|---|---|---|---|---|
| 0.3.0 | 651,904 | 656,000 | 1,048,576 | | |
| 0.6.0 | 1,399,664 | 1,499,776 | 1,400,000 | | |
| 0.7.0 | 1,420,416 | 1,520,256 | 1,430,000 | 681,266 | 700,000 |
| 0.8.0 | 1,515,672 | 1,618,560 | 1,530,000 | 728,388 | 750,000 |
| 0.9.0 | 1,544,072 | 1,643,136 | 1,560,000 | 743,164 | 750,000 |
| 0.10.0 | 1,592,688 | 1,696,384 | 1,600,000 | 767,166 | 800,000 |
