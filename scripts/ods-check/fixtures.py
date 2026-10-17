#!/usr/bin/env python3
"""fixtures.py OUT: writes the ODS test fixtures, built by hand from the
specification (ISO/IEC 26300, ODF 1.3) as LibreOffice writes the parts, into
OUT (tests/fixtures/ods). The expected CSVs beside them are LibreOffice's own
export of every sheet (refs.sh's filter), made once and checked in:

    soffice --headless --convert-to 'csv:Text - txt - csv (StarCalc):44,34,76,1,,0,false,true,true,false,false,-1' \
        --outdir OUT OUT/*.ods

then renamed from <name>-<sheet>.csv to <name>.<sheet>.csv."""

import sys
import zipfile
from pathlib import Path

NS = (
    'xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" '
    'xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" '
    'xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" '
    'xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" '
    'xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" '
    'xmlns:number="urn:oasis:names:tc:opendocument:xmlns:datastyle:1.0" '
    'xmlns:xlink="http://www.w3.org/1999/xlink" '
    'xmlns:dc="http://purl.org/dc/elements/1.1/" '
    'xmlns:calcext="urn:org:documentfoundation:names:experimental:calc:xmlns:calcext:1.0" '
    'xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" '
    'office:version="1.3"'
)

MANIFEST = """<?xml version="1.0" encoding="UTF-8"?>
<manifest:manifest xmlns:manifest="urn:oasis:names:tc:opendocument:xmlns:manifest:1.0" manifest:version="1.3"><manifest:file-entry manifest:full-path="/" manifest:version="1.3" manifest:media-type="application/vnd.oasis.opendocument.spreadsheet"/><manifest:file-entry manifest:full-path="content.xml" manifest:media-type="text/xml"/><manifest:file-entry manifest:full-path="styles.xml" manifest:media-type="text/xml"/></manifest:manifest>"""


def package(path, automatic, body, common=""):
    content = (
        f'<?xml version="1.0" encoding="UTF-8"?>\n<office:document-content {NS}>'
        f"<office:automatic-styles>{automatic}</office:automatic-styles>"
        f"<office:body><office:spreadsheet>{body}</office:spreadsheet></office:body>"
        "</office:document-content>"
    )
    styles = (
        f'<?xml version="1.0" encoding="UTF-8"?>\n<office:document-styles {NS}>'
        f'<office:styles><style:default-style style:family="table-cell"><style:text-properties fo:language="de" fo:country="DE"/></style:default-style>'
        f'<style:style style:name="Default" style:family="table-cell"/>{common}</office:styles>'
        "</office:document-styles>"
    )
    with zipfile.ZipFile(path, "w") as archive:
        # The media type first and stored, as the format requires.
        archive.writestr(
            zipfile.ZipInfo("mimetype"),
            "application/vnd.oasis.opendocument.spreadsheet",
            compress_type=zipfile.ZIP_STORED,
        )
        archive.writestr("content.xml", content, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("styles.xml", styles, compress_type=zipfile.ZIP_DEFLATED)
        archive.writestr("META-INF/manifest.xml", MANIFEST, compress_type=zipfile.ZIP_DEFLATED)


def cell(value_type=None, text=None, style=None, **attributes):
    parts = ["<table:table-cell"]
    if style:
        parts.append(f' table:style-name="{style}"')
    if value_type:
        parts.append(f' office:value-type="{value_type}" calcext:value-type="{value_type}"')
    for key, value in attributes.items():
        parts.append(f' {key.replace("__", ":").replace("_", "-")}="{value}"')
    if text is None:
        return "".join(parts) + "/>"
    return "".join(parts) + f"><text:p>{text}</text:p></table:table-cell>"


def row(*cells, repeat=None):
    attribute = f' table:number-rows-repeated="{repeat}"' if repeat else ""
    return f"<table:table-row{attribute}>{''.join(cells)}</table:table-row>"


DATA_STYLES = """
<number:number-style style:name="N2"><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/></number:number-style>
<number:percentage-style style:name="N3"><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1"/><number:text>%</number:text></number:percentage-style>
<number:currency-style style:name="N4P0" style:volatile="true"><number:currency-symbol>$</number:currency-symbol><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/></number:currency-style>
<number:currency-style style:name="N4"><style:text-properties fo:color="#ff0000"/><number:text>-</number:text><number:currency-symbol>$</number:currency-symbol><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/><style:map style:condition="value()&gt;=0" style:apply-style-name="N4P0"/></number:currency-style>
<number:number-style style:name="N5"><number:scientific-number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:min-exponent-digits="2" number:forced-exponent-sign="true"/></number:number-style>
<number:number-style style:name="N6"><number:fraction number:min-integer-digits="0" number:min-numerator-digits="1" number:min-denominator-digits="1"/></number:number-style>
<number:date-style style:name="N7"><number:year number:style="long"/><number:text>-</number:text><number:month number:style="long"/><number:text>-</number:text><number:day number:style="long"/></number:date-style>
<number:date-style style:name="N8" number:automatic-order="true"><number:day/><number:text>.</number:text><number:month/><number:text>.</number:text><number:year/></number:date-style>
<number:date-style style:name="N9"><number:day-of-week number:style="long"/><number:text>, </number:text><number:month number:style="long" number:textual="true"/><number:text> </number:text><number:day/><number:text>, </number:text><number:year number:style="long"/></number:date-style>
<number:time-style style:name="N10" number:truncate-on-overflow="false"><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text>:</number:text><number:seconds number:style="long"/></number:time-style>
<number:date-style style:name="N11"><number:month number:style="long"/><number:text>/</number:text><number:day number:style="long"/><number:text>/</number:text><number:year/><number:text> </number:text><number:hours number:style="long"/><number:text>:</number:text><number:minutes number:style="long"/><number:text> </number:text><number:am-pm/></number:date-style>
<number:number-style style:name="N12P0" style:volatile="true"><number:number number:decimal-places="0" number:min-decimal-places="0" number:min-integer-digits="1" number:grouping="true"/></number:number-style>
<number:number-style style:name="N12P1" style:volatile="true"><number:number number:decimal-places="0" number:min-decimal-places="0" number:min-integer-digits="1"/></number:number-style>
<number:text-style style:name="N12"><number:text>[low]</number:text><style:map style:condition="value()&gt;=0.5" style:apply-style-name="N12P0"/><style:map style:condition="value()=0" style:apply-style-name="N12P1"/></number:text-style>
<number:number-style style:name="N13" number:language="de" number:country="DE"><number:number number:decimal-places="2" number:min-decimal-places="2" number:min-integer-digits="1" number:grouping="true"/></number:number-style>
<number:number-style style:name="N14"><number:text>Nr. </number:text><number:number number:decimal-places="0" number:min-decimal-places="0" number:min-integer-digits="6" loext:max-blank-integer-digits="2"/></number:number-style>
<number:text-style style:name="N15"><number:text> </number:text><number:text-content/><number:text> </number:text></number:text-style>
<number:boolean-style style:name="N16"><number:boolean/></number:boolean-style>
"""

CELL_STYLES = "".join(
    f'<style:style style:name="ce{n}" style:family="table-cell" style:parent-style-name="Default" style:data-style-name="N{n}"/>'
    for n in range(2, 17)
)


def types(path):
    rows = [
        row(cell("string", "Kind"), cell("string", "Shown")),
        row(cell("string", "text"), cell("string", "A &lt; B &amp; C")),
        row(cell("string", "spaces"), cell("string", 'two<text:s/> spaces<text:tab/>tab')),
        row(cell("string", "lines"), '<table:table-cell office:value-type="string"><text:p>first</text:p><text:p>second<text:line-break/>third</text:p></table:table-cell>'),
        row(cell("string", "link"), '<table:table-cell office:value-type="string"><text:p>see <text:a xlink:href="https://example.com">the site</text:a></text:p><office:annotation><dc:creator>A</dc:creator><text:p>not shown</text:p></office:annotation></table:table-cell>'),
        row(cell("string", "general"), cell("float", "1234.5678", office__value="1234.5678")),
        row(cell("string", "no shown text"), cell("float", None, office__value="0.125")),
        row(cell("string", "grouped"), cell("float", "1234567.891", style="ce2", office__value="1234567.891")),
        row(cell("string", "percent"), cell("percentage", "x", style="ce3", office__value="0.256")),
        row(cell("string", "currency"), cell("currency", "x", style="ce4", office__value="1234.5", office__currency="USD")),
        row(cell("string", "currency, negative"), cell("currency", "x", style="ce4", office__value="-1234.5", office__currency="USD")),
        row(cell("string", "scientific"), cell("float", "x", style="ce5", office__value="12345")),
        row(cell("string", "fraction"), cell("float", "x", style="ce6", office__value="1.5")),
        row(cell("string", "ISO date"), cell("date", "x", style="ce7", office__date_value="2024-01-05")),
        row(cell("string", "locale-order date"), cell("date", "5.1.24", style="ce8", office__date_value="2024-01-05")),
        row(cell("string", "long date"), cell("date", "x", style="ce9", office__date_value="2022-10-29")),
        row(cell("string", "duration"), cell("time", "x", style="ce10", office__time_value="PT36H00M00S")),
        row(cell("string", "date and time"), cell("date", "x", style="ce11", office__date_value="2023-11-22T18:04:59.99")),
        row(cell("string", "low"), cell("float", "x", style="ce12", office__value="0.2")),
        row(cell("string", "low, zero"), cell("float", "x", style="ce12", office__value="0")),
        row(cell("string", "low, large"), cell("float", "x", style="ce12", office__value="1234")),
        row(cell("string", "German format"), cell("float", "1.234,50", style="ce13", office__value="1234.5")),
        row(cell("string", "blank digits"), cell("float", "x", style="ce14", office__value="42")),
        row(cell("string", "text format"), cell("string", "padded", style="ce15")),
        row(cell("string", "boolean"), cell("boolean", "WAHR", office__boolean_value="true")),
        row(cell("string", "boolean as number"), cell("boolean", "x", office__boolean_value="-1")),
    ]
    body = (
        '<table:table table:name="Types"><table:table-column table:number-columns-repeated="1024"/>'
        + "".join(rows)
        + row('<table:table-cell table:number-columns-repeated="1024"/>', repeat=1048546)
        + "</table:table>"
    )
    package(path, "", body, DATA_STYLES + CELL_STYLES)


def sheets(path):
    first = (
        '<table:table table:name="Sales">'
        '<table:table-column table:number-columns-repeated="2"/>'
        '<table:table-column table:default-cell-style-name="ce7"/>'
        '<table:table-column table:number-columns-repeated="16381"/>'
        + row(cell("string", "Region"), cell("string", "Units"), cell("string", "Day"))
        # A merged heading over two columns, its covered cell holding text.
        + row(
            '<table:table-cell office:value-type="string" table:number-columns-spanned="2" table:number-rows-spanned="1"><text:p>North</text:p></table:table-cell>',
            '<table:covered-table-cell office:value-type="string"><text:p>hidden</text:p></table:covered-table-cell>',
            cell("date", "x", office__date_value="2024-03-01"),
        )
        # Three identical rows written once.
        + row(cell("string", "East"), cell("float", "7", office__value="7"), cell("date", "x", office__date_value="2024-03-02"), repeat=3)
        + row('<table:table-cell table:number-columns-repeated="3"/>', repeat=2)
        # A cell repeated across three columns.
        + row('<table:table-cell office:value-type="float" office:value="1" table:number-columns-repeated="3"><text:p>1</text:p></table:table-cell>')
        + row('<table:table-cell table:number-columns-repeated="16384"/>', repeat=1048568)
        + "</table:table>"
    )
    second = (
        '<table:table table:name="Costs &amp; more">'
        "<table:table-column/>"
        + row(cell("string", "Item"), cell("string", "Cost"))
        + row(cell("string", "Paper"), cell("float", "x", style="ce2", office__value="12.5"))
        + "</table:table>"
    )
    empty = '<table:table table:name="Empty"><table:table-column/><table:table-row><table:table-cell/></table:table-row></table:table>'
    package(path, "", first + second + empty, DATA_STYLES + CELL_STYLES)


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    types(out / "types.ods")
    sheets(out / "sheets.ods")


if __name__ == "__main__":
    main()
