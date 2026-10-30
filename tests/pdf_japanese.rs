//! Japanese PDF text: every fixture in tests/fixtures/pdf-japanese reads
//! as the text it was written from, line for line (blank lines left out).
//! The fixtures come from `make.py` beside them: the Shift_JIS, EUC-JP,
//! and Unicode CMaps, vertical writing, an Identity-H font in Adobe-Japan1
//! without ToUnicode (as the Mac writes Hiragino), glyphs set one by one
//! down columns, a ToUnicode map that gives radicals for ideographs, and
//! placeholder glyphs whose text is in `/ActualText`.

use std::fs;
use std::path::PathBuf;

use sublime::io::pdf::text::write_pdf_text;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf-japanese")
}

fn lines(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && *line != "\x0c")
        .map(|line| line.trim_matches('\x0c').to_string())
        .filter(|line| !line.is_empty())
        .collect()
}

#[test]
fn every_japanese_pdf_reads_as_written() {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir())
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "pdf"))
        .collect();
    paths.sort();
    assert_eq!(paths.len(), 8, "the eight fixtures make.py writes");
    for path in paths {
        let bytes = fs::read(&path).expect("read the PDF");
        let mut out = Vec::new();
        write_pdf_text(&bytes, None, &mut out).expect("reads");
        let text = String::from_utf8(out).expect("UTF-8");
        let expected = fs::read_to_string(path.with_extension("txt")).expect("text beside the PDF");
        assert_eq!(lines(&text), lines(&expected), "{}", path.display());
    }
}
