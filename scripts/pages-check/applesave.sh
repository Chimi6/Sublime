#!/bin/bash
# applesave.sh <in.docx> <out.pages>: Pages imports a Word file and saves it as .pages (menu-driven).
HERE="$(cd "$(dirname "$0")" && pwd)"; ROOT="$(cd "$HERE/../.." && pwd)"
SC="${PAGES_CHECK_DIR:-${TMPDIR:-/tmp}/pages-check}"; CORPUS="${PAGES_CHECK_CORPUS:-$HOME/Downloads}"; mkdir -p "$SC"
[ -x "$SC/render" ] || swiftc -O "$HERE/render.swift" -o "$SC/render"
in="$1"; out="$2"; rm -rf "$out"
osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1; sleep 1; killall Pages 2>/dev/null; sleep 1
open -a Pages "$in" --args -ApplePersistenceIgnoreState YES
for i in $(seq 1 40); do sleep 1; w=$(osascript -e 'tell application "System Events" to tell process "Pages" to count windows' 2>/dev/null); [ "${w:-0}" -gt 0 ] && break; done
sleep 3
osascript <<OSA >/dev/null 2>&1
tell application "Pages" to activate
delay 1
tell application "System Events" to tell process "Pages"
  try
    click button "OK" of sheet 1 of front window
    delay 1
  end try
  keystroke "s" using {command down}
  delay 2
  keystroke "g" using {command down, shift down}
  delay 1
  keystroke "$out"
  delay 0.5
  key code 36
  delay 1
  key code 36
end tell
OSA
for i in $(seq 1 30); do [ -e "$out" ] && break; sleep 1; done
sleep 2; osascript -e 'tell application "Pages" to quit saving no' >/dev/null 2>&1; sleep 1; killall Pages 2>/dev/null
[ -e "$out" ] && echo OK || echo FAILED
