//! Pages -> Markdown, HTML, and text through the document model's event
//! projection: structure the fixtures are known to hold must come out as
//! Markdown structure.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::io::html::HtmlWriter;
use sublime::io::markdown::MarkdownWriter;
use sublime::io::pages::{Package, Scope, read_document};
use sublime::io::text::TextWriter;

fn document(name: &str) -> sublime::document::Document {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(format!("{name}.pages"));
    let bytes = std::fs::read(&path).expect("fixture readable");
    let package = Package::read_scope(&bytes, Scope::Document).expect("package reads");
    read_document(&package)
}

fn markdown(name: &str) -> String {
    let document = document(name);
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn html(name: &str) -> String {
    let document = document(name);
    let mut output = Vec::new();
    let mut writer = HtmlWriter::streaming(&mut output);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn text(name: &str) -> String {
    let document = document(name);
    let mut output = Vec::new();
    let mut writer = TextWriter::streaming(&mut output);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

#[test]
fn headings_and_inline_formatting() {
    let markdown = markdown("text-styles");
    assert!(
        markdown
            .starts_with("# Text Styles\n\n*Every character-level attribute Pages can carry*\n")
    );
    assert!(markdown.contains("\n### Heading Three\n"));
    // Formatting is marked up with the whitespace outside the delimiters,
    // and a mark the next run keeps stays open across it.
    assert!(markdown.contains(
        "Plain, **bold,** *italic, **bold italic,*** underlined, ~~struck through,~~ red,"
    ));
    assert!(markdown.contains("A run in the *Emphasis* character style and one in **Strong**."));
}

#[test]
fn lists_nest_by_level() {
    let markdown = markdown("lists");
    assert!(markdown.contains(
        "- First bullet\n- Second bullet\n  - Nested bullet under the second\n    - Third level bullet\n- Third bullet\n"
    ));
    assert!(markdown.contains(
        "1. First number\n   1. Letter a under one\n   2. Letter b under one\n      1. Roman i under b\n2. Second number\n"
    ));
    assert!(markdown.contains("1. Restarted at one\n2. Two\n"));
    assert!(markdown.contains("10. Ten\n11. Eleven\n"));
}

#[test]
fn tables_keep_their_grid() {
    let markdown = markdown("table");
    assert!(markdown.contains("| Alpha | data | 42.50 |\n"));
    // Merged cells keep the grid as empty cells; a multi-paragraph cell
    // is one line.
    assert!(markdown.contains("| Spans two columns |  | Single |\n"));
    assert!(markdown.contains("|  | b3 | c3 |\n"));
    assert!(markdown.contains("| Multi-line cell second paragraph | Empty next |  |\n"));
    let html = html("table");
    assert!(html.contains("<h1>Tables</h1>"));
    assert!(html.contains("<th align=\"left\"><strong>Name</strong></th>"));
    assert!(html.contains("<td align=\"left\">Alpha</td>"));
}

#[test]
fn links_footnotes_and_images() {
    let links = markdown("links");
    assert!(links.contains("A bare URL as text: <https://example.org/plain> and [www\\.example.net](http://www.example.net) without a scheme."));
    assert!(links.contains("A [link to a web page](https://example.com/path?query=1%23fragment) and an [email link](mailto:someone@example.com)."));
    let notes = markdown("notes");
    assert!(notes.contains("A sentence with a footnote[^1] and another[^2] and an endnote[^3]."));
    assert!(notes.contains("\n[^1]: The first footnote.\n"));
    assert!(notes.contains("Tracked changes: inserted words and unchanged words."));
    let images = markdown("images");
    assert!(images.contains("Before ![inline.png](image1-31.png) after."));
}

#[test]
fn text_boxes_follow_the_body_and_text_is_plain() {
    let markdown = markdown("native-objects");
    assert!(markdown.ends_with("Group A\n\nGroup B\n\nA lone shape with text.\n"));
    let text = text("text-styles");
    assert!(text.starts_with(
        "Text Styles\n\nEvery character-level attribute Pages can carry\n\nHeading One\n"
    ));
    assert!(text.contains("Plain, bold, italic, bold italic, underlined, struck through, red,"));
}
