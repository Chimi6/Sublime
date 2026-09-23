#!/usr/bin/env bash
# Builds sublime and the bench harness in release, generates inputs, and
# compares ours against the crate pipelines. Prints pass/fail against the
# targets in DOCS/BENCHMARKS.md.
# Usage: bench/run.sh [rows]   (default 10000000 rows, roughly 1 GB CSV)
set -euo pipefail
cd "$(dirname "$0")/.."

rows="${1:-10000000}"
data="bench/data"
mkdir -p "$data"

echo "== building"
cargo build --release --quiet
(cd bench && cargo build --release --quiet)
sublime="target/release/sublime"
bench="bench/target/release/sublime-bench"

echo "== generating ${rows} rows"
[ -f "$data/big.csv" ] || "$bench" gen-csv "$rows" "$data/big.csv"
[ -f "$data/big.json" ] || "$bench" gen-json "$rows" "$data/big.json"
printf 'a,b\n1,2\n' > "$data/tiny.csv"
csv_bytes="$(wc -c < "$data/big.csv" | tr -d ' ')"
json_bytes="$(wc -c < "$data/big.json" | tr -d ' ')"

# time_cmd <label> <command...> : prints "label seconds max_rss_kb"
time_cmd() {
  local label="$1"; shift
  local best_seconds=""
  local rss=""
  local results=()
  for _ in 1 2 3; do
    local start end
    start="$(date +%s.%N)"
    /usr/bin/time -f "%M" -o "$data/rss.txt" "$@"
    end="$(date +%s.%N)"
    results+=("$(echo "$end - $start" | bc -l)")
    rss="$(cat "$data/rss.txt")"
  done
  local sorted
  sorted="$(printf '%s\n' "${results[@]}" | sort -n)"
  best_seconds="$(echo "$sorted" | sed -n 2p)"
  echo "$label $best_seconds $rss"
}

echo "== running"
ours_c2j="$(time_cmd ours-csv-json "$sublime" -q convert "$data/big.csv" "$data/out1.json")"
crates_c2j="$(time_cmd crates-csv-json "$bench" crates-csv-json "$data/big.csv" "$data/out2.json")"
ours_j2c="$(time_cmd ours-json-csv "$sublime" -q convert "$data/big.json" "$data/out3.csv")"
crates_j2c="$(time_cmd crates-json-csv "$bench" crates-json-csv "$data/big.json" "$data/out4.csv")"

echo "== stdin memory"
/usr/bin/time -f "%M" -o "$data/rss.txt" sh -c "$sublime -q convert - --from csv --to json < $data/big.csv > $data/out5.json"
stdin_rss="$(cat "$data/rss.txt")"

echo "== startup"
start="$(date +%s.%N)"
for _ in $(seq 1 100); do "$sublime" -q convert "$data/tiny.csv" --to json > /dev/null; done
end="$(date +%s.%N)"
startup_ms="$(echo "($end - $start) * 1000 / 100" | bc -l)"

echo "== binary size"
size_bytes="$(wc -c < "$sublime" | tr -d ' ')"
if command -v rustup >/dev/null && rustup target list --installed | grep -q x86_64-unknown-linux-musl; then
  cargo build --release --quiet --target x86_64-unknown-linux-musl
  size_bytes="$(wc -c < target/x86_64-unknown-linux-musl/release/sublime | tr -d ' ')"
  echo "measured musl binary"
else
  echo "musl target not installed; measured gnu binary (install with: rustup target add x86_64-unknown-linux-musl)"
fi

mbps() { echo "scale=1; $1 / $2 / 1048576" | bc -l; }
seconds_of() { echo "$1" | awk '{print $2}'; }
rss_of() { echo "$1" | awk '{print $3}'; }

ours_c2j_s="$(seconds_of "$ours_c2j")";   crates_c2j_s="$(seconds_of "$crates_c2j")"
ours_j2c_s="$(seconds_of "$ours_j2c")";   crates_j2c_s="$(seconds_of "$crates_j2c")"

pass() { if [ "$1" = "1" ]; then echo PASS; else echo FAIL; fi; }

echo
echo "| Target | Ours | Reference | Result |"
echo "|---|---|---|---|"
echo "| CSV -> JSON throughput (MB/s) | $(mbps "$csv_bytes" "$ours_c2j_s") | $(mbps "$csv_bytes" "$crates_c2j_s") | $(pass "$(echo "$ours_c2j_s <= $crates_c2j_s" | bc -l)") |"
echo "| JSON -> CSV throughput (MB/s) | $(mbps "$json_bytes" "$ours_j2c_s") | $(mbps "$json_bytes" "$crates_j2c_s") | $(pass "$(echo "$ours_j2c_s <= $crates_j2c_s" | bc -l)") |"
echo "| Peak RSS CSV -> JSON file (MB) | $(echo "scale=1; $(rss_of "$ours_c2j") / 1024" | bc -l) | < 16 | $(pass "$(echo "$(rss_of "$ours_c2j") < 16384" | bc -l)") |"
echo "| Peak RSS JSON -> CSV file (MB) | $(echo "scale=1; $(rss_of "$ours_j2c") / 1024" | bc -l) | < 16 | $(pass "$(echo "$(rss_of "$ours_j2c") < 16384" | bc -l)") |"
echo "| Peak RSS CSV -> JSON stdin (MB) | $(echo "scale=1; $stdin_rss / 1024" | bc -l) | < 16 | $(pass "$(echo "$stdin_rss < 16384" | bc -l)") |"
echo "| Binary size (bytes) | $size_bytes | < 1048576 | $(pass "$(echo "$size_bytes < 1048576" | bc -l)") |"
echo "| Startup (ms, 1 KB file) | $(printf '%.3f' "$startup_ms") | < 1 | $(pass "$(echo "$startup_ms < 1" | bc -l)") |"
echo
echo "commit: $(git rev-parse --short HEAD)"
echo "machine: $(uname -srm), $(nproc) cpus"
