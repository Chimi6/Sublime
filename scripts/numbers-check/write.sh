#!/bin/bash
# write.sh: our Numbers writer against Numbers itself. Rewrites each test
# document (numbers -> xlsx -> numbers, through our reader and writer), opens
# every result in Numbers, exports it to CSV, and reports per document the
# load-time problems Numbers logs (repairs, upgrades, assertions), crashes,
# and whether Numbers' export matches what our reader reads from the same
# file. Run check.sh first (it fetches the documents). macOS with Numbers;
# not run in CI. Numbers opens each document while it runs.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${NUMBERS_CHECK_DIR:-${TMPDIR:-/tmp}/numbers-check}"; B="$ROOT/target/release/sublime"
rm -rf "$SC/write"; mkdir -p "$SC/write"
pattern='process == "Numbers" AND (eventMessage CONTAINS[c] "repair" OR eventMessage CONTAINS[c] "modified during read" OR eventMessage CONTAINS[c] "Assertion failure" OR eventMessage CONTAINS[c] "No object")'
# Crash reports newer than this marker are this run's.
touch "$SC/write/.start"
clean=0; total=0
for names in "$SC/names"/*.json; do
  n=$(basename "$names" .json); f="$SC/numbers-parser/tests/data/$n.numbers"
  [ -e "$f" ] || continue
  out="$SC/write/$n"; mkdir -p "$out"
  "$B" -q convert "$f" "$out/via.xlsx" 2>/dev/null; "$B" -q convert "$out/via.xlsx" "$out/ours.numbers" 2>/dev/null || continue
  total=$((total+1))
  /usr/bin/log stream --style compact --predicate "$pattern" > "$out/log.txt" 2>&1 & logger=$!
  sleep 1
  osascript -e "with timeout of 90 seconds
    tell application \"Numbers\"
      set d to open POSIX file \"$out/ours.numbers\"
      export d to POSIX file \"$out/app\" as CSV
      close d saving no
    end tell
  end timeout" >/dev/null 2>&1 || {
    osascript -e 'tell application "System Events" to tell process "Numbers" to click button 1 of window 1' >/dev/null 2>&1
    osascript -e 'with timeout of 30 seconds
      tell application "Numbers" to close every document saving no
    end timeout' >/dev/null 2>&1
  }
  sleep 1; kill $logger 2>/dev/null; wait $logger 2>/dev/null
  issues=$(grep -c "Numbers\[" "$out/log.txt")
  "$B" -q convert "$out/ours.numbers" "$out/read.csv" 2>/dev/null
  same=$(python3 "$HERE/same.py" "$out")
  [ "$issues" = 0 ] && [ "$same" = same ] && clean=$((clean+1))
  [ "$issues" = 0 ] && [ "$same" = same ] || echo "$n: $issues log issues, export $same"
done
crashes=$(find ~/Library/Logs/DiagnosticReports -name 'Numbers*' -newer "$SC/write/.start" 2>/dev/null | wc -l | tr -d ' ')
echo "$clean/$total documents open cleanly in Numbers with matching exports; $crashes crashes"
