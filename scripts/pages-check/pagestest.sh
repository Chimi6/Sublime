#!/bin/bash
# Usage: pagestest.sh <file.pages> [pdf-out]
# Opens a file in Pages, reports OPENED / CRASHED / DAMAGED, optionally exports PDF.
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
f="$1"; pdf="$2"
osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1; sleep 1; killall Pages 2>/dev/null; sleep 1
before=$(ls ~/Library/Logs/DiagnosticReports/Pages*.ips 2>/dev/null | wc -l)
open -a Pages "$f"
for i in $(seq 1 20); do
  sleep 1
  after=$(ls ~/Library/Logs/DiagnosticReports/Pages*.ips 2>/dev/null | wc -l)
  if [ "$after" -gt "$before" ]; then echo "CRASHED"; ls -t ~/Library/Logs/DiagnosticReports/Pages*.ips | head -1; exit 2; fi
  n=$(osascript -e 'tell application "Pages" to count documents' 2>/dev/null)
  if [ "${n:-0}" -gt 0 ]; then
    if [ -n "$pdf" ]; then
      rm -f "$pdf"
      for j in $(seq 1 15); do
        osascript -e "tell application \"Pages\" to export front document to POSIX file \"$pdf\" as PDF" >/dev/null 2>&1
        [ -s "$pdf" ] && break; sleep 2
      done
    fi
    echo "OPENED"; osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1; exit 0
  fi
done
# no doc and no crash: probably a "damaged" alert
osascript -e 'tell application "System Events" to tell process "Pages" to get value of static text of window 1' 2>/dev/null | head -c 300; echo
echo "NOT-OPENED"; killall Pages 2>/dev/null; exit 3
