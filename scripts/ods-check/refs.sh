#!/bin/bash
# refs.sh DIR: LibreOffice's CSV export of every sheet of every .ods in
# DIR/src, shown as LibreOffice shows it, into DIR/ref/<name>-<sheet>.csv.
DIR="$1"; mkdir -p "$DIR/ref"
soffice --headless --convert-to 'csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,-1' \
  --outdir "$DIR/ref" "$DIR"/src/*.ods > "$DIR/ref.log" 2>&1
echo "refs: $(ls "$DIR/ref" | wc -l | tr -d ' ') sheets from $(ls "$DIR/src" | wc -l | tr -d ' ') files"
