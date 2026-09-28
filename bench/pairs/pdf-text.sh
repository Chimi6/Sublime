#!/usr/bin/env bash
# PDF -> text pair. Sourced by bench/run.sh. Inputs, generated once with
# PyMuPDF (python3 with fitz) and Ghostscript: a 300-page report set in
# an embedded TrueType font (Type0, Identity-H, ToUnicode), the same
# report as Ghostscript rewrites it (CID fonts in its own units), and a
# 300-page text in the base-14 Helvetica (a simple WinAnsi font without
# widths). References: poppler's pdftotext (C) and the pdf-extract crate
# (lopdf underneath). Throughput counts the PDF's bytes.

run_pair() {
  if [ ! -f "$data/xstory.pdf" ] || [ ! -f "$data/xbase14.pdf" ]; then
    echo "== generating the PDFs" >&2
    python3 - "$data" <<'PY' || { echo "python3 with PyMuPDF (fitz) is needed to generate the inputs" >&2; return 1; }
import sys, fitz
data = sys.argv[1]
words = ("the quick brown fox jumps over a lazy dog while measurements of every "
         "quarter rose and costs held steady across all sites visited twice").split()
def paragraph(seed, length=90):
    return " ".join(words[(seed * 7 + index * 3) % len(words)] for index in range(length)) + "."
html = []
for section in range(1, 301):
    html.append(f"<h2>Section {section}</h2>")
    for block in range(4):
        html.append(f"<p>{paragraph(section * 4 + block)}</p>")
    html.append("<ul>" + "".join(f"<li>{paragraph(section + item, 8)}</li>" for item in range(3)) + "</ul>")
story = fitz.Story(html="".join(html), user_css="body { font-family: sans-serif; }")
writer = fitz.DocumentWriter(f"{data}/xstory.pdf")
more = True
while more:
    device = writer.begin_page(fitz.paper_rect("a4"))
    more, _ = story.place(fitz.Rect(72, 72, 523, 770))
    story.draw(device)
    writer.end_page()
writer.close()
doc = fitz.open()
for number in range(300):
    page = doc.new_page(width=595, height=842)
    y = 72
    for line in range(48):
        page.insert_text((72, y), paragraph(number * 48 + line, 12), fontsize=10, fontname="helv")
        y += 14
doc.save(f"{data}/xbase14.pdf", garbage=3, deflate=True)
PY
  fi
  if [ ! -f "$data/xgs.pdf" ]; then
    gs -q -dNOPAUSE -dBATCH -sDEVICE=pdfwrite -sOutputFile="$data/xgs.pdf" "$data/xstory.pdf" \
      || { echo "Ghostscript is needed for the rewritten input" >&2; return 1; }
  fi

  echo "== running" >&2
  for shape in story gs base14; do
    local pdf="$data/x$shape.pdf" pdf_bytes ours crates poppler
    pdf_bytes="$(wc -c < "$pdf" | tr -d ' ')"
    ours="$(time_cmd ours "$sublime" -q convert "$pdf" "$data/out-$shape-ours.txt")"
    crates="$(time_cmd crates "$bench" pdf-text crates-pdf-text "$pdf" "$data/out-$shape-crates.txt")"
    poppler="$(time_cmd poppler pdftotext -enc UTF-8 "$pdf" "$data/out-$shape-poppler.txt")"
    row "pdf -> text, ${shape} ($(mb "$pdf_bytes") MB, 300 pages): throughput (MB/s of PDF)" "$(mbps "$pdf_bytes" "$(seconds_of "$ours")")" "$(mbps "$pdf_bytes" "$(seconds_of "$crates")") (pdf-extract); $(mbps "$pdf_bytes" "$(seconds_of "$poppler")") (pdftotext)" "$(pass "$(echo "$(seconds_of "$ours") <= $(seconds_of "$crates") && $(seconds_of "$ours") <= $(seconds_of "$poppler")" | bc -l)")"
    row "pdf -> text, ${shape}: peak memory (MB)" "$(rss_mb "$(rss_of "$ours")")" "$(rss_mb "$(rss_of "$crates")") (pdf-extract); $(rss_mb "$(rss_of "$poppler")") (pdftotext)" "$(pass "$(echo "$(rss_of "$ours") <= $(rss_of "$crates") && $(rss_of "$ours") <= $(rss_of "$poppler")" | bc -l)")"
    local ours_words poppler_words crates_words
    ours_words="$(wc -w < "$data/out-$shape-ours.txt" | tr -d ' ')"
    poppler_words="$(wc -w < "$data/out-$shape-poppler.txt" | tr -d ' ')"
    crates_words="$(wc -w < "$data/out-$shape-crates.txt" | tr -d ' ')"
    row "pdf -> text, ${shape}: words out [extra]" "$ours_words" "$crates_words (pdf-extract); $poppler_words (pdftotext)" "n/a"
    rm -f "$data/out-$shape-"*
  done
}
