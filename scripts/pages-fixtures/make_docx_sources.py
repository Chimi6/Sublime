#!/usr/bin/env python3
"""Writes the Word documents that become the Pages fixtures.

Pages imports Word documents faithfully (styles, lists, footnotes, comments,
tracked changes, sections, columns, headers and footers, images, tables), so
a deterministic Word file per feature, opened and saved by Pages itself, is
the cheapest way to a complete, rerunnable fixture set. Standard library
only; the OOXML is written by hand.

Usage: scripts/pages-fixtures/make_docx_sources.py [tests/fixtures/pages/sources]
"""

import struct
import sys
import zlib
import zipfile
from pathlib import Path

W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'
R = 'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
WP = 'xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing"'
A = 'xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main"'
PIC = 'xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture"'
NS = f"{W} {R} {WP} {A} {PIC}"

REL_NS = "http://schemas.openxmlformats.org/officeDocument/2006/relationships"
PKG_REL_NS = "http://schemas.openxmlformats.org/package/2006/relationships"
W14 = 'xmlns:w14="http://schemas.microsoft.com/office/word/2010/wordml"'
W15 = 'xmlns:w15="http://schemas.microsoft.com/office/word/2012/wordml"'
COMMENTS_EXT_REL = "http://schemas.microsoft.com/office/2011/relationships/commentsExtended"


# ----- PNG without any library -----

def png(width, height, seed):
    """An RGB gradient PNG, different per seed, so images are told apart."""
    rows = bytearray()
    for y in range(height):
        rows.append(0)
        for x in range(width):
            rows += bytes(((x * 255) // max(width - 1, 1), (y * 255) // max(height - 1, 1), (seed * 60) % 256))

    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body) & 0xFFFFFFFF)

    header = struct.pack(">IIBBBBB", width, height, 8, 2, 0, 0, 0)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", header) + chunk(b"IDAT", zlib.compress(bytes(rows))) + chunk(b"IEND", b"")


# ----- XML helpers -----

def esc(text):
    return text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;")


LINK_STYLE = '<w:rStyle w:val="Hyperlink"/>'
HEADER_STYLE = '<w:pStyle w:val="Header"/>'
FOOTER_STYLE = '<w:pStyle w:val="Footer"/>'


def run(text, props=""):
    rpr = f"<w:rPr>{props}</w:rPr>" if props else ""
    return f'<w:r>{rpr}<w:t xml:space="preserve">{esc(text)}</w:t></w:r>'


def para(content, props=""):
    ppr = f"<w:pPr>{props}</w:pPr>" if props else ""
    return f"<w:p>{ppr}{content}</w:p>"


def styled(style, text):
    return para(run(text), f'<w:pStyle w:val="{style}"/>')


def body(text):
    return para(run(text))


def list_item(num_id, level, text):
    return para(run(text), f'<w:pStyle w:val="ListParagraph"/><w:numPr><w:ilvl w:val="{level}"/><w:numId w:val="{num_id}"/></w:numPr>')


def tab():
    return "<w:r><w:tab/></w:r>"


def tab_stops(stops):
    """stops: list of (val, pos_twips, leader|None)."""
    tags = "".join(
        f'<w:tab w:val="{val}" w:pos="{pos}"' + (f' w:leader="{leader}"' if leader else "") + "/>"
        for val, pos, leader in stops
    )
    return f"<w:tabs>{tags}</w:tabs>"


def hyperlink(rel_id, text):
    return f'<w:hyperlink r:id="{rel_id}">{run(text, LINK_STYLE)}</w:hyperlink>'


def anchor_link(anchor, text):
    return f'<w:hyperlink w:anchor="{anchor}">{run(text, LINK_STYLE)}</w:hyperlink>'


def inline_image(rel_id, image_id, cx, cy, name):
    return (
        f'<w:r><w:drawing><wp:inline distT="0" distB="0" distL="0" distR="0">'
        f'<wp:extent cx="{cx}" cy="{cy}"/><wp:docPr id="{image_id}" name="{name}"/>'
        f'{graphic(rel_id, image_id, cx, cy, name)}</wp:inline></w:drawing></w:r>'
    )


def floating_image(rel_id, image_id, cx, cy, name):
    return (
        f'<w:r><w:drawing><wp:anchor distT="0" distB="0" distL="114300" distR="114300" simplePos="0" '
        f'relativeHeight="2" behindDoc="0" locked="0" layoutInCell="1" allowOverlap="1">'
        f'<wp:simplePos x="0" y="0"/>'
        f'<wp:positionH relativeFrom="margin"><wp:align>right</wp:align></wp:positionH>'
        f'<wp:positionV relativeFrom="paragraph"><wp:posOffset>0</wp:posOffset></wp:positionV>'
        f'<wp:extent cx="{cx}" cy="{cy}"/><wp:wrapSquare wrapText="bothSides"/>'
        f'<wp:docPr id="{image_id}" name="{name}"/>'
        f'{graphic(rel_id, image_id, cx, cy, name)}</wp:anchor></w:drawing></w:r>'
    )


def graphic(rel_id, image_id, cx, cy, name):
    return (
        '<a:graphic><a:graphicData uri="http://schemas.openxmlformats.org/drawingml/2006/picture">'
        f'<pic:pic><pic:nvPicPr><pic:cNvPr id="{image_id}" name="{name}"/><pic:cNvPicPr/></pic:nvPicPr>'
        f'<pic:blipFill><a:blip r:embed="{rel_id}"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill>'
        f'<pic:spPr><a:xfrm><a:off x="0" y="0"/><a:ext cx="{cx}" cy="{cy}"/></a:xfrm>'
        '<a:prstGeom prst="rect"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>'
    )


def cell(content, width=3000, extra=""):
    return f'<w:tc><w:tcPr><w:tcW w:w="{width}" w:type="dxa"/>{extra}</w:tcPr>{content}</w:tc>'


def sect(extra="", cols=1, header_rel=None, footer_rel=None, break_type=None):
    parts = []
    if break_type:
        parts.append(f'<w:type w:val="{break_type}"/>')
    if header_rel:
        parts.append(f'<w:headerReference w:type="default" r:id="{header_rel}"/>')
    if footer_rel:
        parts.append(f'<w:footerReference w:type="default" r:id="{footer_rel}"/>')
    parts.append('<w:pgSz w:w="12240" w:h="15840"/>')
    parts.append('<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>')
    parts.append(f'<w:cols w:num="{cols}" w:space="708"/>')
    parts.append(extra)
    return f"<w:sectPr>{''.join(parts)}</w:sectPr>"


# ----- shared parts -----

STYLES = f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:styles {W}>
<w:docDefaults><w:rPrDefault><w:rPr><w:rFonts w:ascii="Helvetica Neue" w:hAnsi="Helvetica Neue"/><w:sz w:val="22"/></w:rPr></w:rPrDefault>
<w:pPrDefault><w:pPr><w:spacing w:after="160" w:line="259" w:lineRule="auto"/></w:pPr></w:pPrDefault></w:docDefaults>
<w:style w:type="paragraph" w:default="1" w:styleId="Normal"><w:name w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Title"><w:name w:val="Title"/><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:b/><w:sz w:val="56"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Subtitle"><w:name w:val="Subtitle"/><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:i/><w:sz w:val="28"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading1"><w:name w:val="heading 1"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="480" w:after="120"/><w:outlineLvl w:val="0"/></w:pPr><w:rPr><w:b/><w:sz w:val="36"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading2"><w:name w:val="heading 2"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="360" w:after="100"/><w:outlineLvl w:val="1"/></w:pPr><w:rPr><w:b/><w:sz w:val="30"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Heading3"><w:name w:val="heading 3"/><w:basedOn w:val="Normal"/><w:pPr><w:keepNext/><w:spacing w:before="240" w:after="80"/><w:outlineLvl w:val="2"/></w:pPr><w:rPr><w:b/><w:i/><w:sz w:val="26"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Quote"><w:name w:val="Quote"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720" w:right="720"/></w:pPr><w:rPr><w:i/><w:color w:val="555555"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="ListParagraph"><w:name w:val="List Paragraph"/><w:basedOn w:val="Normal"/><w:pPr><w:ind w:left="720"/><w:contextualSpacing/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="Caption"><w:name w:val="caption"/><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr><w:rPr><w:i/><w:sz w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="Header"><w:name w:val="header"/><w:basedOn w:val="Normal"/></w:style>
<w:style w:type="paragraph" w:styleId="Footer"><w:name w:val="footer"/><w:basedOn w:val="Normal"/><w:pPr><w:jc w:val="center"/></w:pPr></w:style>
<w:style w:type="paragraph" w:styleId="FootnoteText"><w:name w:val="footnote text"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="18"/></w:rPr></w:style>
<w:style w:type="paragraph" w:styleId="EndnoteText"><w:name w:val="endnote text"/><w:basedOn w:val="Normal"/><w:rPr><w:sz w:val="18"/></w:rPr></w:style>
<w:style w:type="character" w:default="1" w:styleId="DefaultParagraphFont"><w:name w:val="Default Paragraph Font"/></w:style>
<w:style w:type="character" w:styleId="Hyperlink"><w:name w:val="Hyperlink"/><w:rPr><w:color w:val="0563C1"/><w:u w:val="single"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Emphasis"><w:name w:val="Emphasis"/><w:rPr><w:i/></w:rPr></w:style>
<w:style w:type="character" w:styleId="Strong"><w:name w:val="Strong"/><w:rPr><w:b/></w:rPr></w:style>
<w:style w:type="character" w:styleId="FootnoteReference"><w:name w:val="footnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>
<w:style w:type="character" w:styleId="EndnoteReference"><w:name w:val="endnote reference"/><w:rPr><w:vertAlign w:val="superscript"/></w:rPr></w:style>
<w:style w:type="table" w:default="1" w:styleId="TableNormal"><w:name w:val="Normal Table"/><w:tblPr><w:tblCellMar><w:left w:w="108" w:type="dxa"/><w:right w:w="108" w:type="dxa"/></w:tblCellMar></w:tblPr></w:style>
<w:style w:type="table" w:styleId="TableGrid"><w:name w:val="Table Grid"/><w:basedOn w:val="TableNormal"/><w:tblPr><w:tblBorders><w:top w:val="single" w:sz="4" w:color="000000"/><w:left w:val="single" w:sz="4" w:color="000000"/><w:bottom w:val="single" w:sz="4" w:color="000000"/><w:right w:val="single" w:sz="4" w:color="000000"/><w:insideH w:val="single" w:sz="4" w:color="000000"/><w:insideV w:val="single" w:sz="4" w:color="000000"/></w:tblBorders></w:tblPr></w:style>
<w:style w:type="paragraph" w:customStyle="1" w:styleId="Callout"><w:name w:val="Callout"/><w:basedOn w:val="Normal"/><w:pPr><w:pBdr><w:top w:val="single" w:sz="8" w:space="6" w:color="BF9000"/><w:left w:val="single" w:sz="8" w:space="6" w:color="BF9000"/><w:bottom w:val="single" w:sz="8" w:space="6" w:color="BF9000"/><w:right w:val="single" w:sz="8" w:space="6" w:color="BF9000"/></w:pBdr><w:shd w:val="clear" w:color="auto" w:fill="FFF2CC"/><w:spacing w:before="120" w:after="120"/><w:ind w:left="360" w:right="360"/></w:pPr><w:rPr><w:i/><w:color w:val="7F6000"/></w:rPr></w:style>
<w:style w:type="character" w:customStyle="1" w:styleId="CodeChar"><w:name w:val="Code Char"/><w:basedOn w:val="DefaultParagraphFont"/><w:rPr><w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/><w:shd w:val="clear" w:color="auto" w:fill="EFEFEF"/><w:color w:val="A31515"/></w:rPr></w:style>
</w:styles>'''


def numbering():
    def level(ilvl, fmt, text, indent):
        return (f'<w:lvl w:ilvl="{ilvl}"><w:start w:val="1"/><w:numFmt w:val="{fmt}"/><w:lvlText w:val="{text}"/>'
                f'<w:lvlJc w:val="left"/><w:pPr><w:ind w:left="{indent}" w:hanging="360"/></w:pPr></w:lvl>')
    bullets = level(0, "bullet", "•", 720) + level(1, "bullet", "◦", 1440) + level(2, "bullet", "▪", 2160)
    decimal = level(0, "decimal", "%1.", 720) + level(1, "lowerLetter", "%2.", 1440) + level(2, "lowerRoman", "%3.", 2160)
    letters = level(0, "upperLetter", "%1)", 720) + level(1, "decimal", "%2)", 1440)
    return f'''<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<w:numbering {W}>
<w:abstractNum w:abstractNumId="0"><w:multiLevelType w:val="hybridMultilevel"/>{bullets}</w:abstractNum>
<w:abstractNum w:abstractNumId="1"><w:multiLevelType w:val="hybridMultilevel"/>{decimal}</w:abstractNum>
<w:abstractNum w:abstractNumId="2"><w:multiLevelType w:val="hybridMultilevel"/>{letters}</w:abstractNum>
<w:num w:numId="1"><w:abstractNumId w:val="0"/></w:num>
<w:num w:numId="2"><w:abstractNumId w:val="1"/></w:num>
<w:num w:numId="3"><w:abstractNumId w:val="2"/></w:num>
<w:num w:numId="4"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="1"/></w:lvlOverride></w:num>
<w:num w:numId="5"><w:abstractNumId w:val="1"/><w:lvlOverride w:ilvl="0"><w:startOverride w:val="10"/></w:lvlOverride></w:num>
</w:numbering>'''


def footnotes_part(notes):
    items = "".join(
        f'<w:footnote w:id="{note_id}"><w:p><w:pPr><w:pStyle w:val="FootnoteText"/></w:pPr>'
        f'<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteRef/></w:r>{run(" " + text)}</w:p></w:footnote>'
        for note_id, text in notes
    )
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:footnotes {W}>'
            '<w:footnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>'
            '<w:footnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>'
            f'{items}</w:footnotes>')


def endnotes_part(notes):
    items = "".join(
        f'<w:endnote w:id="{note_id}"><w:p><w:pPr><w:pStyle w:val="EndnoteText"/></w:pPr>'
        f'<w:r><w:rPr><w:rStyle w:val="EndnoteReference"/></w:rPr><w:endnoteRef/></w:r>{run(" " + text)}</w:p></w:endnote>'
        for note_id, text in notes
    )
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:endnotes {W}>'
            '<w:endnote w:type="separator" w:id="-1"><w:p><w:r><w:separator/></w:r></w:p></w:endnote>'
            '<w:endnote w:type="continuationSeparator" w:id="0"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:endnote>'
            f'{items}</w:endnotes>')


def footnote_ref(note_id):
    return f'<w:r><w:rPr><w:rStyle w:val="FootnoteReference"/></w:rPr><w:footnoteReference w:id="{note_id}"/></w:r>'


def endnote_ref(note_id):
    return f'<w:r><w:rPr><w:rStyle w:val="EndnoteReference"/></w:rPr><w:endnoteReference w:id="{note_id}"/></w:r>'


def comments_part(comments):
    items = "".join(
        f'<w:comment w:id="{comment_id}" w:author="{esc(author)}" w:date="2026-09-23T12:00:00Z" w:initials="{esc(author[:2])}">'
        f'<w:p>{run(text)}</w:p></w:comment>'
        for comment_id, author, text in comments
    )
    return f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments {W}>{items}</w:comments>'


def commented(comment_id, content):
    return (f'<w:commentRangeStart w:id="{comment_id}"/>{content}<w:commentRangeEnd w:id="{comment_id}"/>'
            f'<w:r><w:commentReference w:id="{comment_id}"/></w:r>')


def header_part(text):
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:hdr {W} {R}>'
            f'{para(run(text), HEADER_STYLE)}</w:hdr>')


def footer_part():
    page_field = ('<w:r><w:fldChar w:fldCharType="begin"/></w:r><w:r><w:instrText xml:space="preserve"> PAGE </w:instrText></w:r>'
                  '<w:r><w:fldChar w:fldCharType="separate"/></w:r><w:r><w:t>1</w:t></w:r><w:r><w:fldChar w:fldCharType="end"/></w:r>')
    return (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:ftr {W} {R}>'
            f'{para(run("Page ") + page_field, FOOTER_STYLE)}</w:ftr>')


# ----- package writer -----

class Docx:
    def __init__(self):
        self.parts = {}
        self.doc_rels = []
        self.content_types = []
        self.media = []
        self.settings_extra = ""

    def rel(self, kind, target, external=False):
        rel_id = f"rId{len(self.doc_rels) + 10}"
        mode = ' TargetMode="External"' if external else ""
        self.doc_rels.append(f'<Relationship Id="{rel_id}" Type="{REL_NS}/{kind}" Target="{target}"{mode}/>')
        return rel_id

    def add_part(self, name, xml, content_type, kind=None):
        self.parts[f"word/{name}"] = xml
        self.content_types.append(f'<Override PartName="/word/{name}" ContentType="{content_type}"/>')
        if kind:
            return self.rel(kind, name)
        return None

    def add_image(self, name, data):
        self.parts[f"word/media/{name}"] = data
        return self.rel("image", f"media/{name}")

    def write(self, path, body_xml):
        self.parts["word/document.xml"] = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {NS}><w:body>{body_xml}</w:body></w:document>'
        self.add_part("styles.xml", STYLES, "application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml", "styles")
        self.add_part("numbering.xml", numbering(), "application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml", "numbering")
        settings = f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:settings {W}>{self.settings_extra}<w:footnotePr><w:footnote w:id="-1"/><w:footnote w:id="0"/></w:footnotePr><w:endnotePr><w:endnote w:id="-1"/><w:endnote w:id="0"/></w:endnotePr></w:settings>'
        self.add_part("settings.xml", settings, "application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml", "settings")
        content_types = ('<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
                         '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
                         '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
                         '<Default Extension="xml" ContentType="application/xml"/>'
                         '<Default Extension="png" ContentType="image/png"/>'
                         '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
                         f'{"".join(self.content_types)}</Types>')
        root_rels = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_REL_NS}">'
                     f'<Relationship Id="rId1" Type="{REL_NS}/officeDocument" Target="word/document.xml"/></Relationships>')
        doc_rels = (f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><Relationships xmlns="{PKG_REL_NS}">'
                    f'{"".join(self.doc_rels)}</Relationships>')
        with zipfile.ZipFile(path, "w", zipfile.ZIP_DEFLATED) as archive:
            archive.writestr("[Content_Types].xml", content_types)
            archive.writestr("_rels/.rels", root_rels)
            archive.writestr("word/_rels/document.xml.rels", doc_rels)
            for name, data in self.parts.items():
                archive.writestr(name, data)


# ----- the documents -----

LOREM = ("Sublime converts files. This paragraph exists so that the paragraph has more than one line "
         "when laid out on a page, which matters for line spacing, indents, and column tests. "
         "It says nothing else of interest.")


def text_styles(out):
    d = Docx()
    parts = [
        styled("Title", "Text Styles"),
        styled("Subtitle", "Every character-level attribute Pages can carry"),
        styled("Heading1", "Heading One"),
        body("Body text under heading one. " + LOREM),
        styled("Heading2", "Heading Two"),
        body("Body text under heading two."),
        styled("Heading3", "Heading Three"),
        para(run("Plain, ") + run("bold, ", "<w:b/>") + run("italic, ", "<w:i/>") + run("bold italic, ", "<w:b/><w:i/>")
             + run("underlined, ", '<w:u w:val="single"/>') + run("struck through, ", "<w:strike/>")
             + run("red, ", '<w:color w:val="FF0000"/>') + run("large, ", '<w:sz w:val="36"/>')
             + run("small, ", '<w:sz w:val="14"/>') + run("monospace, ", '<w:rFonts w:ascii="Courier New" w:hAnsi="Courier New"/>')
             + run("highlighted, ", '<w:highlight w:val="yellow"/>') + run("super", "") + run("script", '<w:vertAlign w:val="superscript"/>')
             + run(", sub", "") + run("script", '<w:vertAlign w:val="subscript"/>') + run(", and small caps.", "<w:smallCaps/>")),
        para(run("A run in the ") + run("Emphasis", '<w:rStyle w:val="Emphasis"/>') + run(" character style and one in ")
             + run("Strong", '<w:rStyle w:val="Strong"/>') + run(".")),
        para(run("Unicode: café, naïve, 日本語, emoji \U0001F600, and an em—dash.")),
        styled("Quote", "A quoted paragraph in the Quote style, indented on both sides."),
        sect(),
    ]
    d.write(out / "text-styles.docx", "".join(parts))


def paragraphs(out):
    d = Docx()
    parts = [
        styled("Heading1", "Paragraph Formatting"),
        para(run("Left aligned. " + LOREM), '<w:jc w:val="left"/>'),
        para(run("Centered. " + LOREM), '<w:jc w:val="center"/>'),
        para(run("Right aligned. " + LOREM), '<w:jc w:val="right"/>'),
        para(run("Justified. " + LOREM), '<w:jc w:val="both"/>'),
        para(run("Double spaced. " + LOREM), '<w:spacing w:line="480" w:lineRule="auto"/>'),
        para(run("Space before 24pt and after 24pt. " + LOREM), '<w:spacing w:before="480" w:after="480"/>'),
        para(run("First line indented half an inch. " + LOREM), '<w:ind w:firstLine="720"/>'),
        para(run("Left indented one inch. " + LOREM), '<w:ind w:left="1440"/>'),
        para(run("Hanging indent. " + LOREM), '<w:ind w:left="720" w:hanging="720"/>'),
        para(run("A line break inside") + "<w:r><w:br/></w:r>" + run("the same paragraph.")),
        para(run("Text before a page break."), ""),
        para('<w:r><w:br w:type="page"/></w:r>'),
        para(run("Text after the page break, on page two.")),
        para(run("Keep with next and widow control on. " + LOREM), '<w:keepNext/><w:widowControl/>'),
        sect(),
    ]
    d.write(out / "paragraphs.docx", "".join(parts))


def lists(out):
    d = Docx()
    parts = [
        styled("Heading1", "Lists"),
        body("A bulleted list:"),
        list_item(1, 0, "First bullet"),
        list_item(1, 0, "Second bullet"),
        list_item(1, 1, "Nested bullet under the second"),
        list_item(1, 2, "Third level bullet"),
        list_item(1, 0, "Third bullet"),
        body("A numbered list with letters and roman numerals below it:"),
        list_item(2, 0, "First number"),
        list_item(2, 1, "Letter a under one"),
        list_item(2, 1, "Letter b under one"),
        list_item(2, 2, "Roman i under b"),
        list_item(2, 0, "Second number"),
        body("A paragraph between two numbered lists; the next list restarts at one:"),
        list_item(4, 0, "Restarted at one"),
        list_item(4, 0, "Two"),
        body("A list starting at ten:"),
        list_item(5, 0, "Ten"),
        list_item(5, 0, "Eleven"),
        body("Upper-case letters with parentheses:"),
        list_item(3, 0, "Letter A"),
        list_item(3, 1, "Number one under A"),
        list_item(3, 0, "Letter B"),
        body("A list item with two paragraphs:"),
        list_item(1, 0, "Item with a continuation"),
        para(run("Continuation paragraph inside the item, not a new bullet."), '<w:pStyle w:val="ListParagraph"/>'),
        list_item(1, 0, "Next item"),
        sect(),
    ]
    d.write(out / "lists.docx", "".join(parts))


def links(out):
    d = Docx()
    web = d.rel("hyperlink", "https://example.com/path?query=1#fragment", external=True)
    mail = d.rel("hyperlink", "mailto:someone@example.com", external=True)
    parts = [
        styled("Heading1", "Links"),
        para(run("A ") + hyperlink(web, "link to a web page") + run(" and an ") + hyperlink(mail, "email link") + run(".")),
        para(run("A bare URL as text: https://example.org/plain and www.example.net without a scheme.")),
        para(run("Jump to the ") + anchor_link("target", "bookmark below") + run(".")),
        body(LOREM),
        para('<w:bookmarkStart w:id="1" w:name="target"/>' + run("This paragraph is the bookmark target.") + '<w:bookmarkEnd w:id="1"/>'),
        para(hyperlink(web, "The whole paragraph is one link")),
        sect(),
    ]
    d.write(out / "links.docx", "".join(parts))


def table(out):
    d = Docx()
    grid = '<w:tblGrid><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/><w:gridCol w:w="3000"/></w:tblGrid>'
    tblpr = '<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="9000" w:type="dxa"/></w:tblPr>'
    right = para(run("42.50"), '<w:jc w:val="right"/>')

    def row(cells, header=False):
        trpr = "<w:trPr><w:tblHeader/></w:trPr>" if header else ""
        return f"<w:tr>{trpr}{''.join(cells)}</w:tr>"

    simple = (
        f"<w:tbl>{tblpr}{grid}"
        + row([cell(para(run("Name", "<w:b/>"))), cell(para(run("Kind", "<w:b/>"))), cell(para(run("Amount", "<w:b/>")))], header=True)
        + row([cell(body("Alpha")), cell(body("data")), cell(right)])
        + row([cell(body("Beta")), cell(para(run("document, ") + run("bold", "<w:b/>"))), cell(para(run("7"), '<w:jc w:val="right"/>'))])
        + "</w:tbl>"
    )
    shaded = '<w:shd w:val="clear" w:color="auto" w:fill="DDEBF7"/>'
    merged = (
        f"<w:tbl>{tblpr}{grid}"
        + row([cell(body("Spans two columns"), 6000, '<w:gridSpan w:val="2"/>' + shaded), cell(body("Single"))])
        + row([cell(body("Spans two rows"), 3000, '<w:vMerge w:val="restart"/>'), cell(body("b2")), cell(body("c2"))])
        + row([cell(para(""), 3000, "<w:vMerge/>"), cell(body("b3")), cell(body("c3"))])
        + row([cell(body("Multi-line cell") + body("second paragraph")), cell(body("Empty next")), cell(para(""))])
        + "</w:tbl>"
    )
    parts = [
        styled("Heading1", "Tables"),
        body("A simple table with a header row and a numeric column:"),
        simple,
        body("A table with merged cells, a shaded cell, and a multi-paragraph cell:"),
        merged,
        body("Text after the tables."),
        sect(),
    ]
    d.write(out / "table.docx", "".join(parts))


def images(out):
    d = Docx()
    one = d.add_image("image1.png", png(96, 64, 1))
    two = d.add_image("image2.png", png(64, 96, 2))
    emu = 9525  # EMU per pixel at 96 dpi
    parts = [
        styled("Heading1", "Images"),
        body("An inline image follows this sentence: ") ,
        para(run("Before ") + inline_image(one, 1, 96 * emu, 64 * emu, "inline.png") + run(" after.")),
        body("A floating image on the right with text wrapping around it. " + LOREM + " " + LOREM),
        para(floating_image(two, 2, 64 * emu, 96 * emu, "floating.png") + run(LOREM)),
        body(LOREM),
        para(inline_image(one, 3, 192 * emu, 128 * emu, "captioned.png"), '<w:jc w:val="center"/>'),
        styled("Caption", "Figure 1: a captioned image, scaled to twice its size"),
        sect(),
    ]
    d.write(out / "images.docx", "".join(parts))


def notes(out):
    d = Docx()
    d.add_part("footnotes.xml", footnotes_part([(1, "The first footnote."), (2, "The second footnote, with a link-free sentence.")]),
               "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml", "footnotes")
    d.add_part("endnotes.xml", endnotes_part([(1, "The only endnote.")]),
               "application/vnd.openxmlformats-officedocument.wordprocessingml.endnotes+xml", "endnotes")
    d.add_part("comments.xml", comments_part([(0, "Reviewer", "This is a comment on the word 'commented'."), (1, "Reviewer", "A second comment on a whole sentence.")]),
               "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml", "comments")
    inserted = '<w:ins w:id="10" w:author="Editor" w:date="2026-09-23T12:00:00Z">' + run("inserted words ") + "</w:ins>"
    deleted = '<w:del w:id="11" w:author="Editor" w:date="2026-09-23T12:00:00Z"><w:r><w:delText xml:space="preserve">deleted words </w:delText></w:r></w:del>'
    parts = [
        styled("Heading1", "Notes, Comments, and Changes"),
        para(run("A sentence with a footnote") + footnote_ref(1) + run(" and another") + footnote_ref(2) + run(" and an endnote") + endnote_ref(1) + run(".")),
        para(run("A ") + commented(0, run("commented")) + run(" word.")),
        para(commented(1, run("This whole sentence carries a comment."))),
        para(run("Tracked changes: ") + inserted + deleted + run("and unchanged words.")),
        sect(),
    ]
    d.write(out / "notes.docx", "".join(parts))


def layout(out):
    d = Docx()
    header_one = d.add_part("header1.xml", header_part("Section one header"), "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml", "header")
    header_two = d.add_part("header2.xml", header_part("Section two header, two columns"), "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml", "header")
    footer = d.add_part("footer1.xml", footer_part(), "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml", "footer")
    section_one = para("", sect(header_rel=header_one, footer_rel=footer))
    parts = [
        styled("Heading1", "Layout"),
        body("Section one: single column, a header, and a footer with the page number. " + LOREM),
        body(LOREM),
        section_one,
        styled("Heading1", "Two Columns"),
        body("Section two flows in two columns. " + LOREM + " " + LOREM),
        body(LOREM + " " + LOREM),
        body(LOREM),
        sect(cols=2, header_rel=header_two, footer_rel=footer, break_type="nextPage"),
    ]
    d.write(out / "layout.docx", "".join(parts))


def everything(out):
    d = Docx()
    web = d.rel("hyperlink", "https://example.com/", external=True)
    one = d.add_image("image1.png", png(120, 80, 3))
    d.add_part("footnotes.xml", footnotes_part([(1, "A footnote in the mixed document.")]),
               "application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml", "footnotes")
    grid = '<w:tblGrid><w:gridCol w:w="4500"/><w:gridCol w:w="4500"/></w:tblGrid>'
    tblpr = '<w:tblPr><w:tblStyle w:val="TableGrid"/><w:tblW w:w="9000" w:type="dxa"/></w:tblPr>'
    tbl = (f"<w:tbl>{tblpr}{grid}<w:tr><w:trPr><w:tblHeader/></w:trPr>{cell(para(run('Feature', '<w:b/>')), 4500)}{cell(para(run('Status', '<w:b/>')), 4500)}</w:tr>"
           f"<w:tr>{cell(body('Headings'), 4500)}{cell(body('present'), 4500)}</w:tr>"
           f"<w:tr>{cell(body('Lists'), 4500)}{cell(body('present'), 4500)}</w:tr></w:tbl>")
    parts = [
        styled("Title", "A Document With Everything"),
        styled("Subtitle", "Mixed features in one file, as real documents are"),
        styled("Heading1", "Introduction"),
        para(run("This document mixes ") + run("bold", "<w:b/>") + run(", ") + run("italic", "<w:i/>") + run(", a ")
             + hyperlink(web, "link") + run(", and a footnote") + footnote_ref(1) + run(" in one paragraph. " + LOREM)),
        styled("Heading2", "A list"),
        list_item(1, 0, "One"),
        list_item(1, 1, "One point one"),
        list_item(1, 0, "Two"),
        styled("Heading2", "A table"),
        tbl,
        styled("Heading2", "An image"),
        para(inline_image(one, 1, 120 * 9525, 80 * 9525, "mixed.png"), '<w:jc w:val="center"/>'),
        styled("Caption", "Figure 1: mixed"),
        styled("Quote", "A closing quotation, indented."),
        body("The end."),
        sect(),
    ]
    d.write(out / "everything.docx", "".join(parts))


def tabs(out):
    d = Docx()
    toc_like = tab_stops([("right", 9000, "dot")])
    decimal = tab_stops([("decimal", 4680, None)])
    columns = tab_stops([("left", 3120, None), ("center", 6240, None), ("right", 9000, None)])
    parts = [
        styled("Heading1", "Tabs and Leaders"),
        body("A right tab with a dotted leader, as in a table of contents or a price list:"),
        para(run("Introduction") + tab() + run("1"), toc_like),
        para(run("Methods and Materials") + tab() + run("12"), toc_like),
        para(run("Conclusion") + tab() + run("120"), toc_like),
        body("A decimal tab aligns numbers on the decimal point:"),
        para(run("Widget") + tab() + run("3.5"), decimal),
        para(run("Gadget") + tab() + run("12.75"), decimal),
        para(run("Sprocket") + tab() + run("100.0"), decimal),
        body("Left, center, and right tab stops across the line:"),
        para(run("left") + tab() + run("center") + tab() + run("right"), columns),
        sect(),
    ]
    d.write(out / "tabs.docx", "".join(parts))


def custom_styles(out):
    d = Docx()
    parts = [
        styled("Heading1", "Custom Styles"),
        body("The paragraph below uses a user-defined paragraph style named Callout, "
             "not one of the styles Pages ships with:"),
        styled("Callout", "This is a callout: a bordered, shaded, indented, italic paragraph "
                          "whose look comes entirely from a custom style definition."),
        body("The next sentence contains an inline span in a user-defined character style "
             "named Code Char:"),
        para(run("Run ") + run("git status", '<w:rStyle w:val="CodeChar"/>')
             + run(" before you commit.")),
        sect(),
    ]
    d.write(out / "custom-styles.docx", "".join(parts))


def rtl(out):
    d = Docx()
    arabic = "مرحبا بالعالم، هذه فقرة عربية تُكتب من اليمين إلى اليسار."
    hebrew = "שלום עולם, זו פסקה בעברית הנכתבת מימין לשמאל."
    parts = [
        styled("Heading1", "Bidirectional Text"),
        body("A right-to-left Arabic paragraph:"),
        para(run(arabic, "<w:rtl/>"), '<w:bidi/><w:jc w:val="right"/>'),
        body("A right-to-left Hebrew paragraph:"),
        para(run(hebrew, "<w:rtl/>"), '<w:bidi/><w:jc w:val="right"/>'),
        body("A left-to-right paragraph with an inline right-to-left phrase:"),
        para(run("The sign read ") + run("مخرج", "<w:rtl/>") + run(" (exit) above the door.")),
        sect(),
    ]
    d.write(out / "rtl.docx", "".join(parts))


def headers(out):
    d = Docx()
    d.settings_extra = "<w:evenAndOddHeaders/>"
    hct = "application/vnd.openxmlformats-officedocument.wordprocessingml.header+xml"
    fct = "application/vnd.openxmlformats-officedocument.wordprocessingml.footer+xml"
    first = d.add_part("hdr_first.xml", header_part("First-page header, shown only on page one"), hct, "header")
    even = d.add_part("hdr_even.xml", header_part("Even-page header, shown on page two"), hct, "header")
    odd = d.add_part("hdr_odd.xml", header_part("Odd-page header, shown on pages one and three"), hct, "header")
    footer = d.add_part("ftr_headers.xml", footer_part(), fct, "footer")
    refs = (f'<w:headerReference w:type="first" r:id="{first}"/>'
            f'<w:headerReference w:type="even" r:id="{even}"/>'
            f'<w:headerReference w:type="default" r:id="{odd}"/>'
            f'<w:footerReference w:type="default" r:id="{footer}"/>')
    sectpr = (f'<w:sectPr>{refs}<w:pgSz w:w="12240" w:h="15840"/>'
              '<w:pgMar w:top="1440" w:right="1440" w:bottom="1440" w:left="1440" w:header="708" w:footer="708" w:gutter="0"/>'
              '<w:cols w:num="1" w:space="708"/><w:titlePg/></w:sectPr>')
    page_break = para('<w:r><w:br w:type="page"/></w:r>')
    parts = [
        styled("Heading1", "Header Variants"),
        body("This section defines a different header for the first page, for even pages, and for "
             "odd pages, so one document shows three headers. The first page suppresses the odd "
             "header in favour of its own. " + LOREM),
        page_break,
        body("Page two carries the even-page header. " + LOREM),
        page_break,
        body("Page three carries the odd-page (default) header. " + LOREM),
        para("", sectpr),
    ]
    d.write(out / "headers.docx", "".join(parts))


def toc(out):
    d = Docx()
    dot = tab_stops([("right", 9000, "dot")])
    begin = '<w:r><w:fldChar w:fldCharType="begin"/></w:r>'
    instr = '<w:r><w:instrText xml:space="preserve"> TOC \\o "1-3" \\z \\u </w:instrText></w:r>'
    sep = '<w:r><w:fldChar w:fldCharType="separate"/></w:r>'
    end = '<w:r><w:fldChar w:fldCharType="end"/></w:r>'

    def entry(text, page, first=False, last=False):
        content = (begin + instr + sep if first else "") + run(text) + tab() + run(page) + (end if last else "")
        return para(content, dot)

    page_break = para('<w:r><w:br w:type="page"/></w:r>')
    parts = [
        styled("Title", "A Document With a Table of Contents"),
        styled("Heading1", "Contents"),
        entry("Introduction", "2", first=True),
        entry("Background", "3"),
        entry("Conclusion", "4", last=True),
        page_break,
        styled("Heading1", "Introduction"),
        body("The introduction begins on its own page so the contents page numbers mean something. " + LOREM),
        page_break,
        styled("Heading1", "Background"),
        body("Background material. " + LOREM),
        styled("Heading2", "A subsection"),
        body("A level-two heading the contents field includes. " + LOREM),
        page_break,
        styled("Heading1", "Conclusion"),
        body("The end. " + LOREM),
        sect(),
    ]
    d.write(out / "toc.docx", "".join(parts))


def revisions(out):
    d = Docx()
    date = "2026-09-23T12:00:00Z"
    # A comment (id 0) and a reply to it (id 1), threaded via commentsExtended.
    comments_xml = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:comments {W} {W14}>'
        f'<w:comment w:id="0" w:author="Reviewer" w:date="{date}" w:initials="RV">'
        f'<w:p w14:paraId="0A000001">{run("Please expand this point.")}</w:p></w:comment>'
        f'<w:comment w:id="1" w:author="Author" w:date="{date}" w:initials="AU">'
        f'<w:p w14:paraId="0A000002">{run("Good idea; I have added a sentence.")}</w:p></w:comment>'
        '</w:comments>')
    d.add_part("comments.xml", comments_xml,
               "application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml", "comments")
    ext_xml = (
        f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w15:commentsEx {W15}>'
        '<w15:commentEx w15:paraId="0A000001" w15:done="0"/>'
        '<w15:commentEx w15:paraId="0A000002" w15:paraIdParent="0A000001" w15:done="0"/>'
        '</w15:commentsEx>')
    d.parts["word/commentsExtended.xml"] = ext_xml
    d.content_types.append('<Override PartName="/word/commentsExtended.xml" '
                           'ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml"/>')
    ext_rid = f"rId{len(d.doc_rels) + 10}"
    d.doc_rels.append(f'<Relationship Id="{ext_rid}" Type="{COMMENTS_EXT_REL}" Target="commentsExtended.xml"/>')

    def thread(content):
        return ('<w:commentRangeStart w:id="0"/><w:commentRangeStart w:id="1"/>' + content
                + '<w:commentRangeEnd w:id="0"/><w:commentRangeEnd w:id="1"/>'
                '<w:r><w:commentReference w:id="0"/></w:r><w:r><w:commentReference w:id="1"/></w:r>')

    fmt_change = ('<w:r><w:rPr><w:b/><w:rPrChange w:id="30" w:author="Editor" w:date="' + date + '">'
                  '<w:rPr/></w:rPrChange></w:rPr><w:t xml:space="preserve">now bold</w:t></w:r>')
    inserted = f'<w:ins w:id="31" w:author="Editor" w:date="{date}">' + run("an inserted clause ") + "</w:ins>"
    parts = [
        styled("Heading1", "Comment Threads and Tracked Formatting"),
        para(run("A reviewer commented on ") + thread(run("this phrase"))
             + run(" and the author replied, forming a two-message thread.")),
        para(run("The words ") + fmt_change + run(" carry a tracked formatting change (not bold to bold), "
             "and here is ") + inserted + run("kept as a tracked insertion.")),
        sect(),
    ]
    d.write(out / "revisions.docx", "".join(parts))


def main():
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "tests/fixtures/pages/sources")
    out.mkdir(parents=True, exist_ok=True)
    (out / "image1.png").write_bytes(png(96, 64, 1))
    for build in (text_styles, paragraphs, lists, links, table, images, notes, layout, everything,
                  tabs, custom_styles, rtl, headers, toc, revisions):
        build(out)
    print("wrote", ", ".join(sorted(path.name for path in out.iterdir())))


if __name__ == "__main__":
    main()
