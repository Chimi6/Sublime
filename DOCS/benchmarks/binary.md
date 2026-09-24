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

### 2026-09-24, 0.6.2 with the WebAssembly module

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
| 0.6.2 | 1,420,416 | 1,520,256 | 1,430,000 | 681,266 | 700,000 |
