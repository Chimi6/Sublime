//! Markdown -> Word through the events bridge. Two oracles: the bridge and
//! the projection agree (a document built from the events reads back to
//! the Markdown our writer gives for the events themselves) across the
//! CommonMark and GFM corpora for the constructs the model represents;
//! and a document covering every construct survives the whole trip
//! through a Word file.

use std::path::PathBuf;

use sublime::document::from_events::DocumentBuilder;
use sublime::document::markdown::emit_events;
use sublime::io::docx::{read_docx, write_docx};
use sublime::io::html::{HtmlWriter, push_html};
use sublime::io::json::{JsonTokenizer, Token};
use sublime::io::markdown::{Event, MarkdownWriter, Options, Parser, Tag, TagEnd, parse_into};

fn fixture(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/markdown");
    path.push(name);
    path
}

struct Example {
    markdown: String,
    number: String,
    section: String,
}

fn load(name: &str) -> Vec<Example> {
    let bytes = std::fs::read(fixture(name)).expect("fixture readable");
    let mut tokenizer = JsonTokenizer::new(&bytes[..]);
    let mut examples = Vec::new();
    tokenizer.expect(Token::BeginArray).unwrap();
    loop {
        match tokenizer.next_token().unwrap() {
            Token::EndArray => break,
            Token::Comma => continue,
            Token::BeginObject => {
                let mut example = Example {
                    markdown: String::new(),
                    number: String::new(),
                    section: String::new(),
                };
                loop {
                    match tokenizer.next_token().unwrap() {
                        Token::EndObject => break,
                        Token::Comma => continue,
                        Token::String => {
                            let key = tokenizer.text().to_string();
                            tokenizer.expect(Token::Colon).unwrap();
                            let value_token = tokenizer.next_token().unwrap();
                            let value = tokenizer.text().to_string();
                            match key.as_str() {
                                "markdown" => example.markdown = value,
                                "example" => example.number = value,
                                "section" => example.section = value,
                                _ => tokenizer.skip_value(value_token).unwrap(),
                            }
                        }
                        other => panic!("unexpected {other:?}"),
                    }
                }
                examples.push(example);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    examples
}

/// The events rendered straight to Markdown by our writer.
fn direct(markdown: &str) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    parse_into(markdown, Options::default(), &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// The events rendered straight to HTML.
fn direct_html(markdown: &str) -> String {
    let mut html = String::new();
    push_html(&mut html, Parser::new(markdown));
    html
}

fn build(markdown: &str) -> sublime::document::Document {
    let mut builder = DocumentBuilder::new();
    parse_into(markdown, Options::default(), &mut builder);
    builder.finish()
}

/// The events built into a document and projected back to Markdown.
fn bridged(markdown: &str) -> String {
    let document = build(markdown);
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// The events built into a document and projected back to HTML.
fn bridged_html(markdown: &str) -> String {
    let document = build(markdown);
    let mut output = Vec::new();
    let mut writer = HtmlWriter::streaming(&mut output);
    emit_events(&document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// Through a Word file as well.
fn through_word(markdown: &str) -> String {
    let document = build(markdown);
    let docx = write_docx(&document, Vec::new()).expect("docx writes");
    let read_back = read_docx(&docx).expect("docx reads");
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(&read_back, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// HTML with what the model does not carry taken out: paragraph tags
/// (Word has no tight or loose lists), link titles, code languages, and
/// the difference between a soft break and a space.
fn normalized(html: &str) -> String {
    let mut text = html.replace("<p>", "").replace("</p>", "");
    while let Some(start) = text.find(" title=\"") {
        let end = text[start + 8..]
            .find('"')
            .map(|index| start + 8 + index + 1);
        match end {
            Some(end) => text.replace_range(start..end, ""),
            None => break,
        }
    }
    while let Some(start) = text.find(" class=\"language-") {
        let end = text[start + 8..]
            .find('"')
            .map(|index| start + 8 + index + 1);
        match end {
            Some(end) => text.replace_range(start..end, ""),
            None => break,
        }
    }
    // Adjacent runs of one formatting are one span, and the order two
    // formattings nest in has no meaning.
    let text = text
        .replace("</strong><strong>", "")
        .replace("</em><em>", "")
        .replace("</del><del>", "")
        .replace("<strong><em>", "<em><strong>")
        .replace("</em></strong>", "</strong></em>");
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Whether an example uses something the model has no form for: raw
/// HTML, images (which become links), task markers, empty headings,
/// blocks other than a paragraph or a list inside a list item, blocks
/// other than paragraphs inside a block quote, empty quotes, items, and
/// links, and quotes that touch.
fn outside_the_model(markdown: &str, html: &str) -> bool {
    const NO_FORM: &[&str] = &[
        "<h1></h1>",
        "<h2></h2>",
        "<h3></h3>",
        "<h4></h4>",
        "<h5></h5>",
        "<h6></h6>",
        "<li>\n<p>",
        "<li>\n<pre",
        "<li>\n<blockquote",
        "<li>\n<hr",
        "<li>\n<h",
        "<li>\n<table",
        "<li><pre",
        "<li><blockquote",
        "<li><hr",
        "<li><h",
        "<blockquote>\n<ul",
        "<blockquote>\n<ol",
        "<blockquote>\n<pre",
        "<blockquote>\n<h",
        "<blockquote>\n<hr",
        "<blockquote>\n<table",
        "<blockquote>\n</blockquote>",
        "</blockquote>\n<blockquote>",
        "<li></li>",
        "\"></a>",
    ];
    if NO_FORM.iter().any(|marker| html.contains(marker)) {
        return true;
    }
    let mut events: Vec<Event<'_>> = Vec::new();
    parse_into(markdown, Options::default(), &mut events);
    // Emphasis nested in itself, and a link inside emphasis, have one
    // flat form in the model.
    let mut emphasis = 0u32;
    let mut strong = 0u32;
    let mut items = 0u32;
    let mut quotes = 0u32;
    for event in &events {
        match event {
            Event::Start(Tag::Item) => items += 1,
            Event::End(TagEnd::Item) => items = items.saturating_sub(1),
            Event::Start(Tag::BlockQuote) if items > 0 => return true,
            Event::Start(Tag::BlockQuote) => quotes += 1,
            Event::End(TagEnd::BlockQuote) => quotes = quotes.saturating_sub(1),
            Event::Start(Tag::CodeBlock(_))
            | Event::Start(Tag::Heading(_))
            | Event::Start(Tag::Table(_))
            | Event::Rule
                if items > 0 || quotes > 0 =>
            {
                return true;
            }
            Event::Start(Tag::List { .. }) if quotes > 0 => return true,
            Event::Html(_)
            | Event::InlineHtml(_)
            | Event::TaskListMarker(_)
            | Event::Start(Tag::Image { .. }) => return true,
            Event::Start(Tag::Emphasis) => {
                emphasis += 1;
                if emphasis > 1 {
                    return true;
                }
            }
            Event::End(TagEnd::Emphasis) => emphasis = emphasis.saturating_sub(1),
            Event::Start(Tag::Strong) => {
                strong += 1;
                if strong > 1 {
                    return true;
                }
            }
            Event::End(TagEnd::Strong) => strong = strong.saturating_sub(1),
            Event::Start(Tag::Link { .. }) if emphasis > 0 || strong > 0 => return true,
            _ => {}
        }
    }
    false
}

fn corpus(name: &str) -> (usize, usize, Vec<String>) {
    let mut checked = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();
    for example in load(name) {
        let expected_html = direct_html(&example.markdown);
        if outside_the_model(&example.markdown, &expected_html) {
            skipped += 1;
            continue;
        }
        checked += 1;
        let expected = normalized(&expected_html);
        let actual = normalized(&bridged_html(&example.markdown));
        if actual != expected {
            failures.push(format!(
                "{name} example {} ({}):\n--- direct ---\n{expected}\n--- bridged ---\n{actual}\n",
                example.number, example.section
            ));
        }
    }
    (checked, skipped, failures)
}

#[test]
fn commonmark_examples_survive_the_bridge() {
    let (checked, skipped, failures) = corpus("commonmark.json");
    assert!(checked > 300, "checked {checked}, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ ({skipped} skipped):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn gfm_examples_survive_the_bridge() {
    let (checked, skipped, failures) = corpus("gfm.json");
    assert!(checked > 10, "checked {checked}, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ ({skipped} skipped):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const EVERYTHING: &str = "# Title

A paragraph with *emphasis*, **strong**, ~~struck~~, `code`, and a [link](https://example.com/a).

## Second heading

> A quote continued.

- one
- two
  - nested
  - again
- three

1. first
2. second

Between the lists.

7. seven
8. eight

```
fn main() {
    println!(\"hi\");
}
```

---

| Left | Right |
|:-----|------:|
| a | 1 |
| b | 2 |

A footnote[^1] here.

[^1]: The note text.
";

#[test]
fn every_construct_survives_the_bridge() {
    let expected = direct(EVERYTHING);
    assert_eq!(bridged(EVERYTHING), expected);
}

#[test]
fn every_construct_survives_a_word_file() {
    let expected = direct(EVERYTHING);
    assert_eq!(through_word(EVERYTHING), expected);
}

#[test]
fn the_word_file_carries_the_named_styles() {
    let mut builder = DocumentBuilder::new();
    parse_into(EVERYTHING, Options::default(), &mut builder);
    let document = builder.finish();
    let docx = write_docx(&document, Vec::new()).expect("docx writes");
    let read_back = read_docx(&docx).expect("docx reads");
    let names: Vec<&str> = read_back
        .styles
        .paragraph
        .iter()
        .map(|style| style.name.as_str())
        .collect();
    for name in [
        "Heading 1",
        "Heading 2",
        "Quote",
        "Source Code",
        "Horizontal Line",
    ] {
        assert!(names.contains(&name), "{name} in {names:?}");
    }
    assert!(
        read_back
            .styles
            .character
            .iter()
            .any(|style| style.name == "Source Text")
    );
    assert_eq!(read_back.footnotes.len(), 1);
}

#[test]
fn images_by_data_uri_are_embedded_and_others_become_links() {
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let markdown =
        format!("![dot](data:image/png;base64,{png})\n\n![remote](https://example.com/x.png)\n");
    let mut builder = DocumentBuilder::new();
    parse_into(&markdown, Options::default(), &mut builder);
    assert!(builder.other_images());
    let document = builder.finish();
    assert_eq!(document.media.len(), 1);
    assert_eq!(document.media[0].name, "image1.png");
    assert_eq!(document.images.len(), 1);
    assert!(
        document
            .links
            .iter()
            .any(|link| link == "https://example.com/x.png")
    );
}
