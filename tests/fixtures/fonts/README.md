# Test fonts

`DroidSans.ttf`: Droid Sans by Google (Steve Matteson, Ascender
Corporation), from Fedora's `google-droid-sans-fonts` package, licensed
under the Apache License, Version 2.0
(https://www.apache.org/licenses/LICENSE-2.0). Used unchanged as a
fixture for the TrueType reader, subsetting, and PDF font embedding;
it covers Latin, Greek, and Cyrillic.

`NotoSansCJKjp-Subset.otf`: a subset of Noto Sans CJK JP Regular
(Copyright 2014-2021 Adobe, with Reserved Font Name 'Source'), from
github.com/notofonts/noto-cjk, licensed under the SIL Open Font License
1.1 (`OFL-NotoSansCJK.txt`). Cut to 44 glyphs with fontTools'
`pyftsubset` (the text `日本語の文章です。ひらがな、カタカナ、漢字を含みます。「テスト」吾輩は猫であるABCabc123`,
no layout features, no hinting); it stays CID-keyed CFF with five font
DICTs and global subroutines. The fixture for CFF subsetting and PDF
embedding of the fonts CJK systems ship.
