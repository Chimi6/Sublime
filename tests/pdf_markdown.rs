//! PDF text as structure: the HTML story fixture (and Ghostscript's
//! rewrite of it, whose fonts state half the size they draw) reads as the
//! Markdown it was made from; HTML carries the same headings and lists;
//! Word holds the same document, read back through the Word reader.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertOptions, Converter, Input};
use sublime::converters::pdf_to_document::{PDF_TO_DOCX, PDF_TO_HTML, PDF_TO_MARKDOWN};
use sublime::document::markdown::emit_events;
use sublime::event::{Context, NullSink};
use sublime::io::docx::read_docx;
use sublime::io::markdown::MarkdownWriter;

const STORY: &str = "# Report Title

An opening paragraph that runs long enough to wrap across more than one line of the text block, so a paragraph is several lines.

## First Section

Section text with bold words in the middle.

- First bullet item
- Second bullet item

## Second Section

1. Numbered one
2. Numbered two

Closing paragraph.
";

fn fixture(name: &str) -> Vec<u8> {
    fs::read(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/pdf-text")
            .join(name),
    )
    .expect(name)
}

fn convert(converter: &dyn Converter, name: &str) -> Vec<u8> {
    let mut input = std::io::Cursor::new(fixture(name));
    let mut output = Vec::new();
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    converter
        .convert(Input::Stream(&mut input), &mut output, &mut context)
        .expect(name);
    output
}

#[test]
fn the_story_reads_as_its_markdown() {
    for name in ["story.pdf", "gs-story.pdf"] {
        let markdown = String::from_utf8(convert(&PDF_TO_MARKDOWN, name)).expect("utf-8");
        assert_eq!(markdown, STORY, "{name}");
    }
}

#[test]
fn html_carries_the_headings_and_lists() {
    let html = String::from_utf8(convert(&PDF_TO_HTML, "story.pdf")).expect("utf-8");
    for part in [
        "<h1>Report Title</h1>",
        "<h2>First Section</h2>",
        "<li>First bullet item</li>",
        "<ol>",
        "<li>Numbered two</li>",
        "<p>Closing paragraph.</p>",
    ] {
        assert!(html.contains(part), "{part} missing from {html}");
    }
}

#[test]
fn word_holds_the_same_document() {
    let docx = convert(&PDF_TO_DOCX, "story.pdf");
    let document = read_docx(&docx).expect("the Word file reads");
    let mut markdown = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut markdown);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    assert_eq!(String::from_utf8(markdown).expect("utf-8"), STORY);
}

/// Numbered section titles set larger are headings, a bold line at body
/// size is a heading below them, a hyphenated word joins, and the
/// footers' page numbers are dropped.
#[test]
fn a_report_reads_with_its_structure() {
    let markdown = String::from_utf8(convert(&PDF_TO_MARKDOWN, "report.pdf")).expect("utf-8");
    assert_eq!(
        markdown,
        "# Annual Report

## 1. Introduction

This report covers the year and its measurements, which were taken monthly.

### Scope of the work

Every site was visited twice.

## 2. Results

Output rose in every quarter. Costs held steady.
"
    );
}
