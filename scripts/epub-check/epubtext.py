#!/usr/bin/env python3
"""Reads EPUB and Word files the same way for every tool being scored.

    epubtext.py text FILE      the text of an EPUB (its spine, in order) or a
                               Word file (its body, notes, and tables)
    epubtext.py counts FILE    notes, pictures, tables, and headings, as JSON

An EPUB is read with Python's own HTML parser, chapter by chapter in its
spine's order, leaving out the navigation document: no converter under test
reads it. A Word file's counts come from its XML; its text from LibreOffice.
"""

import json
import re
import subprocess
import sys
import tempfile
import zipfile
from html.parser import HTMLParser
from pathlib import Path
from posixpath import dirname, join, normpath
from urllib.parse import unquote


class Text(HTMLParser):
    SKIP = {"script", "style", "head", "title"}
    BLOCK = {"p", "div", "li", "h1", "h2", "h3", "h4", "h5", "h6", "tr", "br",
             "section", "aside", "blockquote", "pre", "td", "th", "dt", "dd"}

    def __init__(self):
        super().__init__(convert_charrefs=True)
        self.out = []
        self.skip = 0
        self.counts = {"notes": 0, "images": 0, "tables": 0, "headings": 0}

    def handle_starttag(self, tag, attrs):
        attrs = dict(attrs)
        words = " ".join(attrs.get(k) or "" for k in ("epub:type", "role", "class")).split()
        if tag in self.SKIP:
            self.skip += 1
        if tag in self.BLOCK:
            self.out.append("\n")
        if tag in ("img", "image"):
            self.counts["images"] += 1
        if tag == "table":
            self.counts["tables"] += 1
        if re.fullmatch(r"h[1-6]", tag):
            self.counts["headings"] += 1
        if {"noteref", "doc-noteref", "footnote-ref", "footnoteref"} & set(words):
            self.counts["notes"] += 1

    def handle_startendtag(self, tag, attrs):
        self.handle_starttag(tag, attrs)
        if tag in self.SKIP:
            self.skip -= 1

    def handle_endtag(self, tag):
        if tag in self.SKIP and self.skip:
            self.skip -= 1
        if tag in self.BLOCK:
            self.out.append("\n")

    def handle_data(self, data):
        if not self.skip:
            self.out.append(data)


def epub(path):
    zf = zipfile.ZipFile(path)
    container = zf.read("META-INF/container.xml").decode("utf-8", "replace")
    opf_path = re.search(r'full-path="([^"]+)"', container).group(1)
    opf = zf.read(opf_path).decode("utf-8", "replace")
    base = dirname(opf_path)
    items = {}
    for item in re.findall(r"<(?:\w+:)?item\b[^>]*>", opf):
        get = lambda name: (re.search(rf'\b{name}="([^"]*)"', item) or [None, ""])[1]
        items[get("id")] = (normpath(join(base, unquote(get("href")))), get("properties"))
    parser = Text()
    pages = []
    for ref in re.findall(r"<(?:\w+:)?itemref\b[^>]*>", opf):
        idref = re.search(r'idref="([^"]*)"', ref).group(1)
        if 'linear="no"' in ref or idref not in items or "nav" in items[idref][1].split():
            continue
        try:
            page = zf.read(items[idref][0]).decode("utf-8", "replace")
        except KeyError:
            continue
        pages.append(page)
        parser.feed(page)
    # The cover, which reading systems show first, unless a page shows it.
    cover = None
    for item in re.findall(r"<(?:\w+:)?item\b[^>]*>", opf):
        if re.search(r'properties="[^"]*\bcover-image\b', item):
            cover = re.search(r'href="([^"]*)"', item).group(1)
    meta = re.search(r'<(?:\w+:)?meta\b[^>]*name="cover"[^>]*content="([^"]*)"', opf)
    if cover is None and meta and meta.group(1) in items:
        cover = items[meta.group(1)][0]
    if cover:
        name = unquote(cover.rsplit("/", 1)[-1])
        if not any(name in page or name.replace(" ", "%20") in page for page in pages):
            parser.counts["images"] += 1
    return "".join(parser.out), parser.counts


def docx_text(path):
    with tempfile.TemporaryDirectory() as directory:
        subprocess.run(["soffice", "--headless", "--convert-to", "txt:Text (encoded):UTF8",
                        "--outdir", directory, str(path)], capture_output=True)
        out = Path(directory) / (Path(path).stem + ".txt")
        text = out.read_text("utf-8", "replace") if out.exists() else ""
    # LibreOffice's text leaves notes out; theirs is part of the document.
    zf = zipfile.ZipFile(path)
    for part in ("word/footnotes.xml", "word/endnotes.xml"):
        if part in zf.namelist():
            xml = zf.read(part).decode("utf-8", "replace")
            for note in re.findall(r"<w:(?:footnote|endnote)\b[^>]*>.*?</w:(?:footnote|endnote)>", xml, re.S):
                text += "\n" + " ".join(re.findall(r"<w:t(?:\s[^>]*)?>([^<]*)</w:t>", note))
    return text


def docx_counts(path):
    zf = zipfile.ZipFile(path)
    names = zf.namelist()
    body = zf.read("word/document.xml").decode("utf-8", "replace")
    notes = 0
    for part in ("word/footnotes.xml", "word/endnotes.xml"):
        if part in names:
            text = zf.read(part).decode("utf-8", "replace")
            notes += len([n for n in re.findall(r'<w:(?:footnote|endnote)\b[^>]*w:id="(-?\d+)"', text) if int(n) > 0])
    styles = zf.read("word/styles.xml").decode("utf-8", "replace") if "word/styles.xml" in names else ""
    heading_ids = set()
    for style in re.findall(r"<w:style\b.*?</w:style>", styles, re.S):
        sid = re.search(r'w:styleId="([^"]*)"', style)
        name = re.search(r'<w:name w:val="([^"]*)"', style)
        outline = re.search(r'<w:outlineLvl w:val="(\d)"', style)
        label = (name.group(1) if name else "").lower()
        if sid and ((outline and int(outline.group(1)) < 9) or label.startswith("heading") or label == "title"):
            heading_ids.add(sid.group(1))
    headings = sum(1 for s in re.findall(r'<w:pStyle w:val="([^"]*)"', body) if s in heading_ids)
    return {
        "notes": notes,
        "images": len(re.findall(r"<a:blip\b", body)) + len(re.findall(r"<v:imagedata\b", body)),
        "tables": len(re.findall(r"<w:tbl>", body)),
        "headings": headings,
    }


def text(path):
    return epub(path)[0] if str(path).endswith(".epub") else docx_text(path)


def counts(path):
    return epub(path)[1] if str(path).endswith(".epub") else docx_counts(path)


if __name__ == "__main__":
    command, path = sys.argv[1], sys.argv[2]
    if command == "text":
        sys.stdout.write(text(path))
    else:
        print(json.dumps(counts(path)))
