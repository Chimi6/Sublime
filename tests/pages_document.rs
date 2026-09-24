//! The Pages document reader against the fixtures: paragraphs, styles,
//! lists, links, footnotes, and breaks must come out as the fixture README
//! says they went in.

use std::path::PathBuf;

use sublime::document::{Alignment, Baseline, Block, Document, Inline, ListLabel, NumberKind};
use sublime::io::pages::{Package, read_document};

fn read(name: &str) -> Document {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(format!("{name}.pages"));
    let bytes = std::fs::read(&path).expect("fixture readable");
    let package = Package::read(&bytes).expect("package reads");
    read_document(&package)
}

fn paragraphs(document: &Document) -> Vec<&sublime::document::Paragraph> {
    document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .collect()
}

fn style_name(document: &Document, paragraph: &sublime::document::Paragraph) -> String {
    paragraph
        .style
        .map(|id| document.styles.paragraph[id].name.clone())
        .unwrap_or_default()
}

#[test]
fn text_styles_paragraphs_and_runs() {
    let document = read("text-styles");
    let texts = document.paragraph_texts();
    assert_eq!(texts[0], "Text Styles");
    assert_eq!(texts[2], "Heading One");
    let paragraphs = paragraphs(&document);
    assert_eq!(style_name(&document, paragraphs[0]), "Title");
    assert_eq!(style_name(&document, paragraphs[1]), "Subtitle");
    assert_eq!(style_name(&document, paragraphs[2]), "Heading");
    // The run paragraph: "Plain, bold, italic, ..." checked as effective
    // formatting, since Pages puts some of it in named character styles.
    let runs = &paragraphs[7].runs;
    let find = |needle: &str| {
        let run = runs
            .iter()
            .find(|run| matches!(&run.content, Inline::Text(text) if text.contains(needle)))
            .unwrap_or_else(|| panic!("no run containing {needle:?}"));
        document.effective_run(paragraphs[7], run)
    };
    assert_eq!(find("bold, ").bold, Some(true));
    assert_eq!(find("italic, ").italic, Some(true));
    assert_eq!(find("underlined").underline, Some(true));
    assert_eq!(find("struck").strike, Some(true));
    assert_eq!(
        find("red").color.map(|color| color.hex()),
        Some("FF0000".to_string())
    );
    assert_eq!(find("large").size, Some(18.0));
    assert!(
        find("monospace")
            .font
            .as_deref()
            .is_some_and(|font| font.contains("Courier"))
    );
    assert_eq!(find("script").baseline, Some(Baseline::Superscript));
    assert_ne!(find("Plain, ").bold, Some(true));
    assert!(runs.iter().any(|run| {
        run.style
            .map(|id| document.styles.character[id].name.as_str())
            == Some("Strong")
    }));
    assert!(runs.iter().all(|run| {
        run.style
            .map(|id| document.styles.character[id].name.as_str())
            != Some("None")
    }));
}

#[test]
fn lists_have_levels_and_formats() {
    let document = read("lists");
    let paragraphs = paragraphs(&document);
    let items: Vec<(String, Option<u8>, bool)> = paragraphs
        .iter()
        .map(|paragraph| {
            (
                paragraph.text(),
                paragraph.list.map(|item| item.level),
                paragraph.list.is_some_and(|item| item.starts_list),
            )
        })
        .collect();
    assert_eq!(items[1], ("A bulleted list:".to_string(), None, false));
    assert_eq!(items[2], ("First bullet".to_string(), Some(0), true));
    assert_eq!(
        items[4],
        ("Nested bullet under the second".to_string(), Some(1), false)
    );
    assert_eq!(items[5], ("Third level bullet".to_string(), Some(2), false));
    assert_eq!(items[8], ("First number".to_string(), Some(0), true));
    let numbered = paragraphs[8].list.unwrap();
    let style = &document.styles.list[numbered.style];
    assert!(
        matches!(&style.levels[0].label, ListLabel::Number(format) if format.kind == NumberKind::Decimal)
    );
    assert!(
        matches!(&style.levels[1].label, ListLabel::Number(format) if format.kind == NumberKind::LowerLetter)
    );
    assert!(
        matches!(&style.levels[2].label, ListLabel::Number(format) if format.kind == NumberKind::LowerRoman)
    );
    let bulleted = &document.styles.list[paragraphs[2].list.unwrap().style];
    assert!(matches!(&bulleted.levels[0].label, ListLabel::Text(_)));
}

#[test]
fn paragraph_formatting_and_breaks() {
    let document = read("paragraphs");
    let paragraphs = paragraphs(&document);
    let by_prefix = |prefix: &str| {
        paragraphs
            .iter()
            .find(|paragraph| paragraph.text().starts_with(prefix))
            .unwrap_or_else(|| panic!("no paragraph starting {prefix:?}"))
    };
    assert_eq!(
        by_prefix("Centered").properties.alignment,
        Some(Alignment::Center)
    );
    assert_eq!(
        by_prefix("Right aligned").properties.alignment,
        Some(Alignment::Right)
    );
    assert_eq!(
        by_prefix("Justified").properties.alignment,
        Some(Alignment::Justify)
    );
    assert_eq!(
        by_prefix("Space before").properties.space_before,
        Some(24.0)
    );
    assert_eq!(
        by_prefix("First line indented")
            .properties
            .first_line_indent,
        Some(36.0)
    );
    assert_eq!(
        by_prefix("Keep with next").properties.keep_with_next,
        Some(true)
    );
    let broken = by_prefix("A line break");
    assert!(
        broken
            .runs
            .iter()
            .any(|run| run.content == Inline::LineBreak)
    );
    assert_eq!(broken.text(), "A line break inside\nthe same paragraph.");
    assert!(by_prefix("Text after the page break").page_break_before);
}

#[test]
fn links_carry_their_targets() {
    let document = read("links");
    let paragraphs = paragraphs(&document);
    let links: Vec<(String, String)> = paragraphs[1]
        .runs
        .iter()
        .filter_map(|run| match (&run.link, &run.content) {
            (Some(url), Inline::Text(text)) => Some((text.clone(), url.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        vec![
            (
                "link to a web page".to_string(),
                "https://example.com/path?query=1%23fragment".to_string()
            ),
            (
                "email link".to_string(),
                "mailto:someone@example.com".to_string()
            ),
        ]
    );
    assert_eq!(
        paragraphs[1].text(),
        "A link to a web page and an email link."
    );
}

#[test]
fn footnotes_are_collected() {
    let document = read("notes");
    let paragraphs = paragraphs(&document);
    let with_notes = paragraphs
        .iter()
        .find(|paragraph| paragraph.text().starts_with("A sentence with a footnote"))
        .expect("footnote paragraph");
    let notes: Vec<usize> = with_notes
        .runs
        .iter()
        .filter_map(|run| match run.content {
            Inline::Footnote(note) => Some(note),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes.len(),
        3,
        "two footnotes and the endnote Pages turned into a footnote"
    );
    let first = &document.footnotes[notes[0]];
    let text = match &first.blocks[0] {
        Block::Paragraph(paragraph) => paragraph.text(),
        _ => String::new(),
    };
    assert_eq!(text, "The first footnote.");
}
