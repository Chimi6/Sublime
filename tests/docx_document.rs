//! Word input against two independent sources: Apple's own Word exports
//! of the Pages fixtures must project to the same text as the Pages
//! documents themselves, and our Word output must read back to the same
//! Markdown the Pages document gives directly.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::document::{Block, Document, Inline, Merge, RevisionKind};
use sublime::io::docx::{read_docx, write_docx};
use sublime::io::markdown::MarkdownWriter;
use sublime::io::pages::{Package, Scope, read_document};
use sublime::io::text::TextWriter;

fn fixture(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(relative);
    path
}

fn pages_document(name: &str) -> Document {
    let bytes = std::fs::read(fixture(&format!("{name}.pages"))).expect("fixture readable");
    let package = Package::read_scope(&bytes, Scope::Document).expect("package reads");
    read_document(&package)
}

fn apple_document(name: &str) -> Document {
    let bytes =
        std::fs::read(fixture(&format!("reference/{name}.docx"))).expect("reference readable");
    read_docx(&bytes).expect("Word package reads")
}

fn markdown(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn text(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = TextWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// Words of a text projection. Internal link targets are left out: Pages
/// names its bookmarks by UUID and Apple's export renames them.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| !word.starts_with("(#"))
        .map(str::to_string)
        .collect()
}

fn first_difference(name: &str, what: &str, ours: &[String], reference: &[String]) {
    for (index, (left, right)) in ours.iter().zip(reference).enumerate() {
        assert_eq!(
            left, right,
            "{name}: {what} differ at word {index} (Word input vs Pages input)"
        );
    }
    assert_eq!(ours.len(), reference.len(), "{name}: {what} word count");
}

/// Fixtures whose Apple export carries the same text as the Pages
/// document, word for word. Left out: `dropcap` (the dropped letter is a
/// paragraph of its own in the export), `ruby` (the base text is
/// dropped), `equations` (Office Math loses operator glyphs), `toc`
/// (an extra empty paragraph), and the `native-*` fixtures (text boxes
/// anchored to different paragraphs).
const TEXT_FIXTURES: &[&str] = &[
    "alt-text",
    "custom-styles",
    "everything",
    "fields",
    "fonts",
    "headers",
    "images",
    "layout",
    "links",
    "lists",
    "metadata",
    "notes",
    "outline-numbering",
    "page-layout",
    "paragraphs",
    "revisions",
    "rtl",
    "table",
    "table-layout",
    "tabs",
    "text-effects",
    "text-styles",
];

/// Every fixture but `equations`: the Word writer puts equations in as
/// text until Office Math is written, so the read-back is text, not math.
const ROUND_TRIP_FIXTURES: &[&str] = &[
    "alt-text",
    "custom-styles",
    "dropcap",
    "everything",
    "fields",
    "fonts",
    "headers",
    "images",
    "layout",
    "links",
    "lists",
    "metadata",
    "native-objects",
    "native-scripted",
    "notes",
    "outline-numbering",
    "page-layout",
    "paragraphs",
    "revisions",
    "rtl",
    "ruby",
    "table",
    "table-layout",
    "tabs",
    "text-effects",
    "text-styles",
    "toc",
];

#[test]
fn apples_export_reads_to_the_pages_text() {
    for name in TEXT_FIXTURES {
        let from_word = words(&text(&apple_document(name)));
        let from_pages = words(&text(&pages_document(name)));
        first_difference(name, "texts", &from_word, &from_pages);
    }
}

#[test]
fn our_word_output_reads_back_to_the_same_markdown() {
    for name in ROUND_TRIP_FIXTURES {
        let mut document = pages_document(name);
        // The Word package names media by index; the reader can only see
        // those names.
        for (index, media) in document.media.iter_mut().enumerate() {
            let extension = media.name.rsplit('.').next().unwrap_or("").to_string();
            media.name = format!("image{}.{extension}", index + 1);
        }
        let direct = markdown(&document);
        let docx = write_docx(&document, Vec::new()).expect("docx writes");
        let read_back = read_docx(&docx).expect("our docx reads");
        let through_word = markdown(&read_back);
        assert_eq!(
            through_word, direct,
            "{name}: pages -> docx -> markdown differs from pages -> markdown"
        );
    }
}

fn body_paragraphs(document: &Document) -> Vec<&sublime::document::Paragraph> {
    let mut paragraphs = Vec::new();
    for section in &document.sections {
        for block in &section.blocks {
            if let Block::Paragraph(paragraph) = block {
                paragraphs.push(paragraph);
            }
        }
    }
    paragraphs
}

#[test]
fn lists_carry_levels_and_kinds() {
    let document = apple_document("lists");
    let items: Vec<(u8, bool)> = body_paragraphs(&document)
        .iter()
        .filter_map(|paragraph| paragraph.list)
        .map(|item| {
            let ordered = document.styles.list[item.style]
                .levels
                .get(usize::from(item.level))
                .is_some_and(|level| {
                    matches!(level.label, sublime::document::ListLabel::Number(_))
                });
            (item.level, ordered)
        })
        .collect();
    assert!(items.len() >= 10, "list paragraphs: {}", items.len());
    assert!(items.contains(&(0, false)), "a top-level bullet");
    assert!(items.contains(&(1, false)), "a nested bullet");
    assert!(items.contains(&(2, false)), "a third-level bullet");
    assert!(items.contains(&(0, true)), "a numbered item");
    let markdown = markdown(&document);
    assert!(markdown.contains("- First bullet\n"), "{markdown}");
    assert!(
        markdown.contains("  - Nested bullet under the second\n"),
        "{markdown}"
    );
    assert!(markdown.contains("1. "), "{markdown}");
}

#[test]
fn tables_keep_their_grid_and_merges() {
    let document = apple_document("table");
    let tables: Vec<&sublime::document::Table> = document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            Block::Paragraph(_) => None,
        })
        .collect();
    assert!(!tables.is_empty());
    let mut saw_left = false;
    let mut saw_above = false;
    for table in &tables {
        let width = table.rows[0].cells.len();
        for row in &table.rows {
            assert_eq!(row.cells.len(), width, "every row spans the grid");
            for cell in &row.cells {
                saw_left |= cell.merge == Merge::Left;
                saw_above |= cell.merge == Merge::Above;
            }
        }
    }
    assert!(saw_left, "a horizontally merged cell");
    assert!(saw_above, "a vertically merged cell");
    assert!(
        tables.iter().any(|table| table.header_rows == 1),
        "a header row"
    );
}

#[test]
fn notes_links_images_and_revisions_are_read() {
    let notes = apple_document("notes");
    assert_eq!(notes.footnotes.len(), 3, "three footnotes");
    let references = body_paragraphs(&notes)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| matches!(run.content, Inline::Footnote(_)))
        .count();
    assert_eq!(references, 3);

    let links = apple_document("links");
    assert!(
        links
            .links
            .iter()
            .any(|target| target.starts_with("https://example.com")),
        "{:?}",
        links.links
    );
    assert!(
        links
            .links
            .iter()
            .any(|target| target.starts_with("mailto:"))
    );
    let markdown = markdown(&links);
    assert!(
        markdown.contains("[link to a web page](https://example.com/"),
        "{markdown}"
    );

    let images = apple_document("images");
    assert!(!images.media.is_empty(), "media parts are loaded");
    assert!(!images.images.is_empty(), "inline images are read");

    // Apple's export keeps insertions and drops deletions.
    let revisions = apple_document("revisions");
    assert!(
        revisions
            .revisions
            .iter()
            .any(|revision| revision.kind == RevisionKind::Insertion)
    );
    let inserted = body_paragraphs(&revisions)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| run.revision.is_some())
        .count();
    assert!(inserted > 0, "runs carry their revision");
}

#[test]
fn headers_and_page_setup_are_read() {
    let document = apple_document("headers");
    let section = &document.sections[0];
    assert!(section.headers.default.is_some(), "a default header");
    assert!(section.footers.default.is_some(), "a default footer");
    assert!(section.headers.first.is_some(), "a first-page header");
    assert!(
        (section.page.width - 612.0).abs() < 0.5,
        "{}",
        section.page.width
    );
    assert!((section.page.margin_left - 72.0).abs() < 0.5);
}

#[test]
fn headings_come_from_outline_levels() {
    let document = apple_document("paragraphs");
    let markdown = markdown(&document);
    assert!(markdown.starts_with("# "), "{markdown}");
}
