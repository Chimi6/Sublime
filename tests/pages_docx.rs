//! Pages -> Word against Apple's own exports: for each fixture the
//! paragraph texts and paragraph style names of our `.docx` must match
//! the reference `.docx` Pages exported from the same document.

use std::borrow::Cow;
use std::path::PathBuf;

use sublime::io::docx::write_docx;
use sublime::io::pages::{Package, read_document};
use sublime::io::xml::{XmlEvent, XmlReader};
use sublime::io::zip::ZipArchive;

fn fixture(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(relative);
    path
}

/// (style id, text) of every paragraph in a document part, footnotes and
/// headers excluded.
fn paragraphs(docx: &[u8]) -> Vec<(String, String)> {
    let archive = ZipArchive::parse(docx).expect("docx is a zip");
    let entry = archive.find("word/document.xml").expect("document part");
    let mut xml = Vec::new();
    archive.read(entry, &mut xml).expect("document part reads");
    let xml = String::from_utf8(xml).expect("utf-8");
    let mut result = Vec::new();
    let mut style = String::new();
    let mut text = String::new();
    let mut in_paragraph = false;
    let mut depth_in_tbl = 0usize;
    let mut in_instruction = false;
    for event in XmlReader::new(&xml) {
        match event {
            XmlEvent::Start { name: "w:p", .. } => {
                in_paragraph = true;
                style.clear();
                text.clear();
            }
            XmlEvent::Start {
                name: "w:pStyle",
                attributes,
                ..
            } if in_paragraph => {
                if let Some((_, value)) = attributes.iter().find(|(key, _)| *key == "w:val") {
                    style = value.to_string();
                }
            }
            XmlEvent::Start { name: "w:tab", .. } if in_paragraph => text.push('\t'),
            XmlEvent::Start { name: "w:br", .. } if in_paragraph => text.push('\n'),
            XmlEvent::Start { name: "w:tbl", .. } => depth_in_tbl += 1,
            XmlEvent::End { name: "w:tbl" } => depth_in_tbl = depth_in_tbl.saturating_sub(1),
            // Apple writes hyperlinks as field codes; the code is not text.
            XmlEvent::Start {
                name: "w:instrText",
                ..
            } => in_instruction = true,
            XmlEvent::End {
                name: "w:instrText",
            } => in_instruction = false,
            XmlEvent::Text(piece) if in_paragraph && !in_instruction => match piece {
                Cow::Borrowed(piece) => text.push_str(piece),
                Cow::Owned(piece) => text.push_str(&piece),
            },
            XmlEvent::End { name: "w:p" } if in_paragraph => {
                in_paragraph = false;
                if depth_in_tbl == 0 {
                    result.push((style.clone(), text.clone()));
                }
            }
            _ => {}
        }
    }
    result
}

fn ours(name: &str) -> Vec<u8> {
    let bytes = std::fs::read(fixture(&format!("{name}.pages"))).expect("fixture readable");
    let package = Package::read(&bytes).expect("package reads");
    let document = read_document(&package);
    write_docx(&document, Vec::new()).expect("docx writes")
}

/// Reports the first paragraph that differs rather than both whole lists.
fn assert_same(name: &str, what: &str, actual: &[String], expected: &[String]) {
    for (index, (left, right)) in actual.iter().zip(expected).enumerate() {
        assert_eq!(
            left, right,
            "{name}: paragraph {index} {what} differ (ours vs Apple)"
        );
    }
    assert_eq!(
        actual.len(),
        expected.len(),
        "{name}: paragraph count ({what})"
    );
}

fn normalize(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Fixtures whose body is text only, where the comparison is exact.
const TEXT_FIXTURES: &[&str] = &[
    "text-styles",
    "paragraphs",
    "lists",
    "links",
    "notes",
    "custom-styles",
    "tabs",
];

#[test]
fn paragraph_texts_match_apples_export() {
    for name in TEXT_FIXTURES {
        let reference =
            std::fs::read(fixture(&format!("reference/{name}.docx"))).expect("reference readable");
        let expected: Vec<String> = paragraphs(&reference)
            .into_iter()
            .map(|(_, text)| normalize(&text))
            .collect();
        let actual: Vec<String> = paragraphs(&ours(name))
            .into_iter()
            .map(|(_, text)| normalize(&text))
            .collect();
        assert_same(name, "texts", &actual, &expected);
    }
}

#[test]
fn paragraph_styles_match_apples_export() {
    for name in TEXT_FIXTURES {
        let reference =
            std::fs::read(fixture(&format!("reference/{name}.docx"))).expect("reference readable");
        let expected: Vec<String> = paragraphs(&reference)
            .into_iter()
            .map(|(style, _)| style)
            .collect();
        let actual: Vec<String> = paragraphs(&ours(name))
            .into_iter()
            .map(|(style, _)| style)
            .collect();
        assert_same(name, "styles", &actual, &expected);
    }
}
