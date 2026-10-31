# Language support

Text is Unicode throughout, so any language in a UTF-8 or UTF-16 file, or
in a format that stores Unicode (Word, Pages, Numbers, Excel, ODF, EPUB,
HTML, XML, JSON, YAML, TOML), converts as written. What differs by
language is reading older encodings, reading text back out of a PDF, and
setting it into a new PDF.

| Script | Text and documents | Legacy encodings read | PDF to text | Text to PDF |
|---|:-:|---|:-:|---|
| Latin, Greek, Cyrillic | ✓ | RTF code pages (437, 850, 1250-1258, Mac Roman) | ✓ | ✓ |
| Japanese | ✓ | Shift_JIS, EUC-JP, ISO-2022-JP in text, Markdown, HTML, XML, CSV, TSV; RTF code page 932 | ✓ incl. vertical writing and fonts with no ToUnicode map | ✓ Japanese line breaking (kinsoku), horizontal only |
| Chinese | ✓ | RTF through `\uN` only | ✓ when the PDF carries a ToUnicode map or a Unicode CMap | ✓ may be set in a Japanese font |
| Korean | ✓ | RTF through `\uN` only | ✓ when the PDF carries a ToUnicode map or a Unicode CMap | ✓ |
| Hebrew, Arabic | ✓ | RTF code pages 1255, 1256 | in drawing order | left to right, isolated letter forms |
| Hindi and other Indic, Thai | ✓ | RTF code page 874 (Thai) | ✓ | glyphs unshaped (no conjuncts); Thai breaks only where a line overflows |

✓ means read back as written in the tests. Text to PDF uses a font
installed on the machine that has the characters (or `--font`); a
character no installed font has is `?` and reported.

## Not yet

- Chinese and Korean legacy encodings (GBK, GB18030, Big5, EUC-KR) in
  text and CSV files: rejected with an error, not misread.
- Chinese and Korean PDFs whose fonts have no ToUnicode map and a
  non-Unicode CMap (the Adobe-GB1, -CNS1, -Korea1 tables).
- Western code page detection in text and CSV files (a Windows-1252 CSV
  is rejected; save it as UTF-8).
- Bidirectional reordering and complex-script shaping in PDF output.
- Vertical Japanese and bold Japanese in PDF output (bold is set in the
  regular weight).

Per-format detail is in [formats/](formats/): `pdf.md`, `csv.md`,
`html.md`, `rtf.md`.
