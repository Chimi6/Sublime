#!/usr/bin/env python3
"""Writes the Japanese PDF fixtures and the text each should read as.

- unijis.pdf, sjis.pdf, euc.pdf, vertical.pdf: reportlab's Japanese CID
  fonts (not embedded, no ToUnicode) through the UniJIS-UCS2-H,
  90ms-RKSJ-H (Shift_JIS), EUC-H, and UniJIS-UCS2-V CMaps.
- identity.pdf: an Identity-H font in Adobe-Japan1 with no ToUnicode, as
  the Mac writes Hiragino: its text comes only from the collection.
- columns.pdf: the same font set one glyph at a time down columns, right
  to left, as the Mac lays out vertical text.
- radicals.pdf: a ToUnicode map that gives Kangxi radicals for the
  ideographs they look like, as Chrome writes some fonts.
- actualtext.pdf: one placeholder glyph drawn for every character, the
  real text in /ActualText spans, as LibreOffice writes Japanese when it
  has no font with the glyphs.

No font is embedded. Needs reportlab; run from the repository root after
`python3 scripts/gen-cjk-tables.py` (the Japan1 table is read from it):

    python3 tests/fixtures/pdf-japanese/make.py
"""

import os
import re

from reportlab.pdfbase import pdfmetrics
from reportlab.pdfbase.cidfonts import CIDFont, UnicodeCIDFont
from reportlab.pdfgen import canvas

HERE = os.path.dirname(os.path.abspath(__file__))
LINES = [
    "日本語の文章です。ひらがな、カタカナ、漢字を含みます。",
    "東京都渋谷区で「テスト」を行いました（２０２６年１０月）。",
    "半角ｶﾀｶﾅとEnglish textも混ざっています！",
]
VERTICAL = [
    "吾輩は猫である。名前はまだ無い。",
    "どこで生れたかとんと見当がつかぬ。",
]


def out(name):
    return os.path.join(HERE, name)


def reportlab_fixtures():
    pdfmetrics.registerFont(UnicodeCIDFont("HeiseiMin-W3"))
    page = canvas.Canvas(out("unijis.pdf"), invariant=1)
    page.setFont("HeiseiMin-W3", 14)
    for index, line in enumerate(LINES):
        page.drawString(50, 750 - 24 * index, line)
    page.save()
    for encoding, codec, name in [("90ms-RKSJ-H", "cp932", "sjis.pdf"), ("EUC-H", "euc_jp", "euc.pdf")]:
        font = CIDFont("HeiseiKakuGo-W5", encoding)
        pdfmetrics.registerFont(font)
        page = canvas.Canvas(out(name), invariant=1)
        page.setFont(font.fontName, 14)
        for index, line in enumerate(LINES):
            page.drawString(50, 750 - 24 * index, line.encode(codec).decode("latin-1"))
        page.save()
    vertical_font = UnicodeCIDFont("HeiseiMin-W3", isVertical=True)
    pdfmetrics.registerFont(vertical_font)
    vertical_name = vertical_font.fontName
    page = canvas.Canvas(out("vertical.pdf"), invariant=1)
    page.setFont(vertical_name, 16)
    for index, line in enumerate(VERTICAL):
        page.drawString(500 - 30 * index, 780, line)
    page.save()


def japan1_cids():
    """Unicode -> the lowest Adobe-Japan1 CID that reads as it."""
    source = open(os.path.join(HERE, "../../../src/io/cjk/tables.rs"), encoding="utf-8").read()
    body = source.split("pub static JAPAN1: [u16;")[1].split("];")[0]
    values = [int(value) for value in re.findall(r"\d+", body.split("=", 1)[1])]
    cids = {}
    for cid, code in enumerate(values):
        if code and chr(code) not in cids:
            cids[chr(code)] = cid
    return cids


def write_pdf(name, font_objects, content, to_unicode=None):
    """A one-page PDF with the font `F1` from `font_objects` (objects 4 on)."""
    objects = [
        "<< /Type /Catalog /Pages 2 0 R >>",
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 4 0 R >> >> /Contents %d 0 R >>" % (4 + len(font_objects)),
    ]
    objects.extend(font_objects)
    data = content.encode("latin-1")
    objects.append("<< /Length %d >>\nstream\n%s\nendstream" % (len(data), content))
    if to_unicode is not None:
        objects.append("<< /Length %d >>\nstream\n%s\nendstream" % (len(to_unicode), to_unicode))
    pdf = "%PDF-1.4\n"
    offsets = []
    for number, body in enumerate(objects, start=1):
        offsets.append(len(pdf.encode("latin-1")))
        pdf += "%d 0 obj\n%s\nendobj\n" % (number, body)
    start = len(pdf.encode("latin-1"))
    pdf += "xref\n0 %d\n0000000000 65535 f \n" % (len(objects) + 1)
    pdf += "".join("%010d 00000 n \n" % offset for offset in offsets)
    pdf += "trailer\n<< /Size %d /Root 1 0 R >>\nstartxref\n%d\n%%%%EOF\n" % (len(objects) + 1, start)
    open(out(name), "wb").write(pdf.encode("latin-1"))


JAPAN1_FONT = [
    "<< /Type /Font /Subtype /Type0 /BaseFont /KozMinPr6N-Regular /Encoding /Identity-H /DescendantFonts [5 0 R] %s>>",
    "<< /Type /Font /Subtype /CIDFontType0 /BaseFont /KozMinPr6N-Regular /CIDSystemInfo << /Registry (Adobe) /Ordering (Japan1) /Supplement 6 >> /DW 1000 /W [1 95 500] /FontDescriptor 6 0 R >>",
    "<< /Type /FontDescriptor /FontName /KozMinPr6N-Regular /Flags 4 /FontBBox [-100 -200 1100 900] /ItalicAngle 0 /Ascent 880 /Descent -120 /CapHeight 740 /StemV 80 >>",
]


def hex_string(cids, text):
    return "<" + "".join("%04X" % cids[ch] for ch in text) + ">"


def handmade_fixtures():
    cids = japan1_cids()
    # Identity-H, no ToUnicode: lines of glyphs, advanced by their widths.
    content = ["BT /F1 14 Tf"]
    for index, line in enumerate(LINES):
        content.append("1 0 0 1 50 %d Tm %s Tj" % (750 - 24 * index, hex_string(cids, line)))
    content.append("ET")
    write_pdf("identity.pdf", [JAPAN1_FONT[0] % "", JAPAN1_FONT[1], JAPAN1_FONT[2]], "\n".join(content))
    # The same glyphs set one by one down columns, right to left.
    content = ["BT /F1 16 Tf"]
    for column, line in enumerate(VERTICAL):
        for row, ch in enumerate(line):
            content.append("1 0 0 1 %d %d Tm %s Tj" % (500 - 30 * column, 780 - 18 * row, hex_string(cids, ch)))
    content.append("ET")
    write_pdf("columns.pdf", [JAPAN1_FONT[0] % "", JAPAN1_FONT[1], JAPAN1_FONT[2]], "\n".join(content))
    # A ToUnicode map giving the Kangxi radical SUN (U+2F47) for 日.
    to_unicode = "\n".join([
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap",
        "/CMapName /Radicals def 1 begincodespacerange <0000> <FFFF> endcodespacerange",
        "2 beginbfchar <%04X> <2F47> <%04X> <672C> endbfchar" % (cids["日"], cids["本"]),
        "endcmap CMapName currentdict /CMap defineresource pop end end",
    ])
    content = "BT /F1 14 Tf 1 0 0 1 50 750 Tm %s Tj ET" % hex_string(cids, "日本")
    write_pdf("radicals.pdf", [JAPAN1_FONT[0] % "/ToUnicode 8 0 R ", JAPAN1_FONT[1], JAPAN1_FONT[2]], content, to_unicode)
    # The same glyph (mapped to 日) for every character, each in a span
    # whose /ActualText is the character itself.
    placeholder = "\n".join([
        "/CIDInit /ProcSet findresource begin 12 dict begin begincmap",
        "/CMapName /Placeholder def 1 begincodespacerange <0000> <FFFF> endcodespacerange",
        "1 beginbfchar <%04X> <65E5> endbfchar" % cids["日"],
        "endcmap CMapName currentdict /CMap defineresource pop end end",
    ])
    content = ["BT /F1 14 Tf"]
    for index, line in enumerate(LINES[:2]):
        for column, ch in enumerate(line):
            units = "".join("%04X" % unit for unit in [int.from_bytes(pair, "big") for pair in [ch.encode("utf-16-be")[i:i + 2] for i in range(0, len(ch.encode("utf-16-be")), 2)]])
            content.append("/Span << /ActualText <FEFF%s> >> BDC 1 0 0 1 %d %d Tm <%04X> Tj EMC" % (units, 50 + 14 * column, 750 - 24 * index, cids["日"]))
    content.append("ET")
    write_pdf("actualtext.pdf", [JAPAN1_FONT[0] % "/ToUnicode 8 0 R ", JAPAN1_FONT[1], JAPAN1_FONT[2]], "\n".join(content), placeholder)


def expectations():
    horizontal = "\n".join(LINES) + "\n"
    for name in ["unijis", "sjis", "euc", "identity"]:
        open(out(name + ".txt"), "w", encoding="utf-8").write(horizontal)
    for name in ["vertical", "columns"]:
        open(out(name + ".txt"), "w", encoding="utf-8").write("\n".join(VERTICAL) + "\n")
    open(out("radicals.txt"), "w", encoding="utf-8").write("日本\n")
    open(out("actualtext.txt"), "w", encoding="utf-8").write("\n".join(LINES[:2]) + "\n")


reportlab_fixtures()
handmade_fixtures()
expectations()
