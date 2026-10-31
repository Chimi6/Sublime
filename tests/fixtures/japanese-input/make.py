#!/usr/bin/env python3
"""Writes Japanese inputs in the legacy encodings and what each reads as.

Plain text in Shift_JIS and EUC-JP, HTML declaring Shift_JIS, CSV in
Shift_JIS (as Excel saves a Japanese CSV) and EUC-JP, XML declaring
Shift_JIS, and RTF as Word writes Japanese: Shift_JIS byte pairs under
\\ansicpg932, and in a \\fcharset128 font inside a Western document.
Python's own codecs make the bytes. Run from the repository root:

    python3 tests/fixtures/japanese-input/make.py
"""
import os

HERE = os.path.dirname(os.path.abspath(__file__))
TEXT = "日本語の文章です。ひらがな、カタカナ、漢字を含みます。半角ｶﾀｶﾅとEnglish！"
ROWS = [["名前", "値段"], ["寿司", "１２００円"], ["ｶﾀｶﾅ", "300"]]


def write(name, data, expected):
    open(os.path.join(HERE, name), "wb").write(data)
    open(os.path.join(HERE, name + ".expected"), "w", encoding="utf-8").write(expected)


def rtf_bytes(text):
    out = ""
    for ch in text:
        encoded = ch.encode("cp932")
        if len(encoded) == 1 and encoded[0] < 0x80:
            out += ch
        else:
            out += "".join(f"\\'{byte:02x}" for byte in encoded)
    return out


write("shift-jis.txt", (TEXT + "\n").encode("cp932"), TEXT)
write("euc-jp.txt", (TEXT + "\n").encode("euc_jp"), TEXT)
html = f'<html><head><meta charset="Shift_JIS"></head><body><p>{TEXT}</p></body></html>\n'
write("shift-jis.html", html.encode("cp932"), TEXT)
csv = "".join(",".join(row) + "\n" for row in ROWS)
write("shift-jis.csv", csv.encode("cp932"), csv)
write("euc-jp.csv", csv.encode("euc_jp"), csv)
xml = '<?xml version="1.0" encoding="Shift_JIS"?>\n<item><name>寿司</name><price>１２００円</price></item>\n'
write("shift-jis.xml", xml.encode("cp932"), '{"item":{"name":"寿司","price":"１２００円"}}')
rtf = "{\\rtf1\\ansi\\ansicpg932\\deff0{\\fonttbl{\\f0\\fnil\\fcharset128 MS Mincho;}}\\f0 " + rtf_bytes(TEXT) + "\\par}"
write("ansicpg932.rtf", rtf.encode("latin-1"), TEXT)
rtf = ("{\\rtf1\\ansi\\ansicpg1252\\deff0{\\fonttbl{\\f0\\froman\\fcharset0 Times;}{\\f1\\fnil\\fcharset128 MS Mincho;}}"
       "\\f0 English then {\\f1 " + rtf_bytes(TEXT) + "}\\par}")
write("fcharset128.rtf", rtf.encode("latin-1"), "English then " + TEXT)
