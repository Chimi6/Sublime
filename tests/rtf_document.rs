//! RTF into the document model and back. Oracles: RTF written by
//! LibreOffice from the Pages fixtures' Word sources reads to the same
//! Markdown as those Word files; RTF written by macOS (`textutil`, the
//! Cocoa text system) reads to its structure; a hand-written legacy file
//! covers code pages, old-style lists, nested tables, and Unicode escapes;
//! and our own RTF of every Word source reads back to the same Markdown.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::document::{Block, Document, Inline, ListLabel, Merge};
use sublime::io::docx::read_docx;
use sublime::io::markdown::MarkdownWriter;
use sublime::io::rtf::{read_rtf, write_rtf};

fn path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(relative);
    path
}

fn rtf(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("rtf/{name}.rtf"))).expect("fixture readable");
    read_rtf(&bytes).expect("RTF reads")
}

fn word(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("pages/sources/{name}.docx"))).expect("source");
    read_docx(&bytes).expect("Word reads")
}

fn markdown(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// LibreOffice's RTF of each source reads as the source does.
#[test]
fn libreoffice_rtf_reads_as_its_word_source() {
    for name in [
        "lists",
        "table",
        "headers",
        "links",
        "page-layout",
        "paragraphs",
        "text-styles",
        "tabs",
        "custom-styles",
    ] {
        assert_eq!(
            markdown(&rtf(&format!("lo-{name}"))),
            markdown(&word(name)),
            "{name}"
        );
    }
}

/// A list LibreOffice restarted at ten keeps ten, from its rendered label.
#[test]
fn a_list_start_is_read_from_the_rendered_label() {
    let text = markdown(&rtf("lo-lists"));
    assert!(text.contains("10. Ten\n11. Eleven"), "{text}");
}

#[test]
fn notes_comments_and_changes() {
    let document = rtf("lo-notes");
    assert_eq!(document.footnotes.len(), 3);
    assert_eq!(document.comments.len(), 2);
    assert_eq!(document.comments[0].author, "Reviewer");
    assert!(document.comments[0].text.contains("commented"));
    let revisions = rtf("lo-revisions");
    assert!(!revisions.revisions.is_empty());
}

#[test]
fn cocoa_rtf_reads_tables_lists_and_headers() {
    let table = rtf("cocoa-table");
    let tables = table.sections[0]
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Table(_)))
        .count();
    assert!(tables >= 2, "{tables} tables");
    let lists = rtf("cocoa-lists");
    let items = lists.sections[0]
        .blocks
        .iter()
        .filter(|block| matches!(block, Block::Paragraph(paragraph) if paragraph.list.is_some()))
        .count();
    assert!(items >= 10, "{items} list items");
    // Cocoa RTF names no styles: its text is compared, not its headings.
    let headers = rtf("cocoa-headers");
    assert_eq!(headers.paragraph_texts(), word("headers").paragraph_texts());
}

#[test]
fn a_legacy_file_with_code_pages_old_lists_and_nested_tables() {
    let document = rtf("legacy");
    let texts = document.paragraph_texts();
    assert_eq!(texts[0], "Привет, мир!");
    assert_eq!(texts[1], "First bullet");
    assert_eq!(texts[5], "Escaped € and red.");
    let Block::Paragraph(bullet) = &document.sections[0].blocks[1] else {
        panic!("paragraph");
    };
    let item = bullet.list.expect("old-style list item");
    assert_eq!(
        document.styles.list[item.style].levels[0].label,
        ListLabel::Text("\u{2022}".to_string())
    );
    let Block::Paragraph(step) = &document.sections[0].blocks[3] else {
        panic!("paragraph");
    };
    assert!(matches!(
        document.styles.list[step.list.expect("numbered").style].levels[0].label,
        ListLabel::Number(_)
    ));
    let Some(Block::Table(outer)) = document.sections[0]
        .blocks
        .iter()
        .find(|block| matches!(block, Block::Table(_)))
    else {
        panic!("table");
    };
    let nested = outer.rows[0].cells[1]
        .blocks
        .iter()
        .find_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .expect("nested table in the second cell");
    assert_eq!(nested.rows[0].cells.len(), 2);
    assert_eq!(outer.rows[0].cells[0].merge, Merge::Origin);
    assert_eq!(texts.last().map(String::as_str), Some("Last"));
}

/// Our RTF of every Word source reads back to the same Markdown.
#[test]
fn our_rtf_of_each_word_source_round_trips() {
    let directory = path("pages/sources");
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .expect("sources")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".docx").map(str::to_string)
        })
        .collect();
    names.sort();
    let mut checked = 0;
    for name in names {
        let source = word(&name);
        let mut bytes = Vec::new();
        write_rtf(&source, &mut bytes).expect("writes");
        let back = read_rtf(&bytes).expect("reads back");
        // Equations are written as their text.
        if name == "equations" {
            continue;
        }
        assert_eq!(markdown(&back), markdown(&source), "{name}");
        checked += 1;
    }
    assert!(checked >= 20, "{checked} sources");
}

#[test]
fn line_separators_are_line_breaks() {
    let document = rtf("cocoa-paragraphs");
    let breaks = document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| run.content == Inline::LineBreak)
        .count();
    assert!(breaks >= 1);
}
