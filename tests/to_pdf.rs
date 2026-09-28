//! Documents to PDF: a Markdown document set as PDF reads back through
//! our own PDF reader as the same Markdown (headings, paragraphs, both
//! kinds of list); a long document fills pages inside the margins and
//! loses no word; the file's structure opens in our reader; links become
//! URI annotations; characters outside WinAnsi are counted as losses;
//! every document format reaches PDF.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertOptions, Converter, Input};
use sublime::converters::pdf_to_document::PDF_TO_MARKDOWN;
use sublime::converters::to_pdf::{
    DOCX_TO_PDF, HTML_TO_PDF, MARKDOWN_TO_PDF, PAGES_TO_PDF, TEXT_TO_PDF,
};
use sublime::event::{Context, NullSink};
use sublime::io::pdf::text::write_pdf_text;

fn fixture(path: &str) -> Vec<u8> {
    fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)).expect(path)
}

fn convert(converter: &dyn Converter, input: &[u8]) -> Vec<u8> {
    convert_with(converter, input, ConvertOptions::default())
}

fn convert_with(converter: &dyn Converter, input: &[u8], options: ConvertOptions) -> Vec<u8> {
    let mut input = std::io::Cursor::new(input.to_vec());
    let mut output = Vec::new();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    converter
        .convert(Input::Stream(&mut input), &mut output, &mut context)
        .expect("converts");
    output
}

fn words(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_string).collect()
}

#[test]
fn markdown_round_trips_through_pdf() {
    let source = fixture("tests/fixtures/to-pdf/structure.md");
    let pdf = convert(&MARKDOWN_TO_PDF, &source);
    let back = String::from_utf8(convert(&PDF_TO_MARKDOWN, &pdf)).expect("utf-8");
    assert_eq!(back, String::from_utf8(source).expect("utf-8"));
}

#[test]
fn a_long_document_fills_pages_and_keeps_every_word() {
    let mut markdown = String::new();
    for section in 1..=40 {
        markdown.push_str(&format!("## Section {section}\n\n"));
        for paragraph in 0..4 {
            for word in 0..60 {
                markdown.push_str(&format!("w{section}x{paragraph}x{word} "));
            }
            markdown.push_str("\n\n");
        }
    }
    let pdf = convert(&MARKDOWN_TO_PDF, markdown.as_bytes());
    let mut text = Vec::new();
    let notes = write_pdf_text(&pdf, None, &mut text).expect("reads");
    assert!(notes.pages > 10, "{} pages", notes.pages);
    let expected: Vec<String> = words(&markdown)
        .into_iter()
        .filter(|word| word != "##")
        .collect();
    assert_eq!(words(&String::from_utf8(text).expect("utf-8")), expected);
    // Every glyph lies inside the one-inch margins of a Letter page.
    sublime::io::pdf::text::for_each_page(&pdf, None, &mut |_, blocks| {
        for line in blocks.iter().flat_map(|block| &block.lines) {
            assert!(
                line.x >= 71.9 && line.y >= 72.0 && line.y <= 720.0,
                "{line:?}"
            );
        }
        Ok(())
    })
    .expect("pages");
}

#[test]
fn links_become_uri_annotations_and_losses_are_counted() {
    // U+0378 is unassigned: no font on any machine has it.
    let pdf = convert(
        &MARKDOWN_TO_PDF,
        "A [site](https://example.com/a) and a mark \u{378}.\n".as_bytes(),
    );
    let text = String::from_utf8_lossy(&pdf);
    assert!(text.contains("/Subtype /Link"));
    assert!(text.contains("/URI (https://example.com/a)"));
    let mut out = Vec::new();
    write_pdf_text(&pdf, None, &mut out).expect("reads");
    let out = String::from_utf8(out).unwrap();
    assert!(out.contains("mark ?"), "{out:?}");
}

#[test]
fn every_document_format_reaches_pdf() {
    let html = b"<h1>Title</h1><p>Some <b>bold</b> text.</p><ul><li>one</li><li>two</li></ul>";
    let text = b"First paragraph.\n\nSecond paragraph.\n";
    let docx = fixture("tests/fixtures/pages/reference/everything.docx");
    let pages = fixture("tests/fixtures/pages/everything.pages");
    for (converter, input, expect) in [
        (&HTML_TO_PDF, html.to_vec(), "Title"),
        (&TEXT_TO_PDF, text.to_vec(), "Second paragraph."),
        (&DOCX_TO_PDF, docx, ""),
        (&PAGES_TO_PDF, pages, ""),
    ] {
        let pdf = convert(converter, &input);
        let mut out = Vec::new();
        write_pdf_text(&pdf, None, &mut out).expect("our PDF reads");
        let out = String::from_utf8(out).unwrap();
        assert!(out.contains(expect), "{}: {out}", converter.name());
        assert!(!out.trim().is_empty(), "{} wrote no text", converter.name());
    }
}

/// A font given for the body sets every character it has, subset and
/// embedded with widths and a ToUnicode map: Greek and Cyrillic read back
/// as written, and the file carries far less than the whole font.
#[test]
fn a_given_font_is_subset_and_embedded() {
    let font = fixture("tests/fixtures/fonts/DroidSans.ttf");
    let options = ConvertOptions {
        font: Some(std::sync::Arc::new(font.clone())),
        ..ConvertOptions::default()
    };
    let text = "# Καλημέρα\n\nПривет, мир. Plain Latin too.\n";
    let pdf = convert_with(&MARKDOWN_TO_PDF, text.as_bytes(), options);
    let raw = String::from_utf8_lossy(&pdf);
    assert!(raw.contains("/Subtype /CIDFontType2"));
    assert!(raw.contains("/FontFile2"));
    assert!(raw.contains("+DroidSans"));
    assert!(
        pdf.len() < font.len() / 4,
        "{} bytes for a {}-byte font",
        pdf.len(),
        font.len()
    );
    let mut out = Vec::new();
    write_pdf_text(&pdf, None, &mut out).expect("reads");
    assert_eq!(
        words(&String::from_utf8(out).unwrap()),
        words("Καλημέρα Привет, мир. Plain Latin too.")
    );
}
