#!/usr/bin/env bash
# Manual benchmark runner. Builds sublime and the harness in release, then
# runs one pair's script and the binary-wide measurements (size, startup).
# Results are recorded by hand in DOCS/benchmarks/<pair>.md; see the README
# there for when to run this and how to write up a result.
#
# Usage: bench/run.sh <pair> [rows]     e.g. bench/run.sh csv-json 10000000
set -euo pipefail
cd "$(dirname "$0")/.."

pair="${1:-}"
rows="${2:-10000000}"
if [ -z "$pair" ] || [ ! -f "bench/pairs/${pair}.sh" ]; then
  echo "usage: bench/run.sh <pair> [rows]" >&2
  echo "pairs: $(ls bench/pairs | sed 's/\.sh$//' | tr '\n' ' ')" >&2
  exit 4
fi

data="bench/data"
mkdir -p "$data"

echo "== building"
cargo build --release --quiet
(cd bench && cargo build --release --quiet)
sublime="target/release/sublime"
bench="bench/target/release/sublime-bench"

# time_cmd <label> <command...> : prints "label median_seconds max_rss_kb"
# over three runs, wall clock per run, peak RSS from GNU time.
time_cmd() {
  local label="$1"; shift
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
  local median
  median="$(printf '%s\n' "${results[@]}" | sort -n | sed -n 2p)"
  echo "$label $median $rss"
}
mbps() { echo "scale=1; $1 / $2 / 1048576" | bc -l; }
seconds_of() { echo "$1" | awk '{print $2}'; }
rss_of() { echo "$1" | awk '{print $3}'; }
rss_mb() { echo "scale=1; $1 / 1024" | bc -l; }
pass() { if [ "$1" = "1" ]; then echo PASS; else echo FAIL; fi; }
row() { echo "| $1 | $2 | $3 | $4 |"; }

echo
echo "| Target | Ours | Reference | Result |"
echo "|---|---|---|---|"

# The pair script defines run_pair, which prints its table rows.
# shellcheck source=/dev/null
source "bench/pairs/${pair}.sh"
run_pair

echo "== binary size" >&2
size_bytes="$(wc -c < "$sublime" | tr -d ' ')"
size_budget="$(tr -d '[:space:]' < size-budget)"
row "Binary size, gnu (bytes)" "$size_bytes" "<= ${size_budget} (size-budget, what CI checks)" "$(pass "$(echo "$size_bytes <= $size_budget" | bc -l)")"
if command -v rustup >/dev/null && rustup target list --installed | grep -q x86_64-unknown-linux-musl; then
  cargo build --release --quiet --target x86_64-unknown-linux-musl
  musl_bytes="$(wc -c < target/x86_64-unknown-linux-musl/release/sublime | tr -d ' ')"
  row "Binary size, musl static (bytes, the release asset)" "$musl_bytes" "recorded" "n/a"
fi

echo "== startup" >&2
printf 'a,b\n1,2\n' > "$data/tiny.csv"
startup_ms="$("$bench" startup "$sublime" "$data/tiny.csv")"
floor_ms="$("$bench" spawn-baseline /bin/true)"
above_floor="$(echo "$startup_ms - $floor_ms" | bc -l)"
row "Startup above spawn floor (ms, 1 KB file)" "$(printf '%.3f' "$above_floor") (spawn $(printf '%.3f' "$startup_ms"), floor $(printf '%.3f' "$floor_ms"))" "< 1" "$(pass "$(echo "$above_floor < 1" | bc -l)")"

echo
echo "commit: $(git rev-parse --short HEAD)"
echo "machine: $(uname -srm), $(nproc) cpus"
