//! HTML input. Two oracles: the HTML our own writer produces for the
//! CommonMark and GFM corpora reads back to events that render to the
//! same HTML (the reader is a fixed point of the writer, up to what HTML
//! itself does not carry); and a page of tag soup reads to the Markdown
//! a person would expect, frozen here.

use std::path::PathBuf;

use sublime::io::html::{HtmlWriter, push_html};
use sublime::io::json::{JsonTokenizer, Token};
use sublime::io::markdown::{Event, MarkdownWriter, Options, Parser, parse_into};
use sublime::io::text::TextWriter;

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

fn markdown_to_html(markdown: &str) -> String {
    let mut html = String::new();
    push_html(&mut html, Parser::new(markdown));
    html
}

fn html_to_html(html: &str) -> String {
    let mut output = Vec::new();
    let mut writer = HtmlWriter::streaming(&mut output);
    sublime::io::html::reader::parse_into(html, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn html_to_markdown(html: &str) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    sublime::io::html::reader::parse_into(html, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn html_to_text(html: &str) -> String {
    let mut output = Vec::new();
    let mut writer = TextWriter::streaming(&mut output);
    sublime::io::html::reader::parse_into(html, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// HTML with what HTML itself does not carry taken out: the difference
/// between a newline and a space in text, and the writer's own newlines
/// between tags.
fn normalized(html: &str) -> String {
    let text = html.split_whitespace().collect::<Vec<_>>().join(" ");
    // Space at the edges of a paragraph is not content in HTML.
    text.replace("<p> ", "<p>").replace(" </p>", "</p>")
}

/// Whether an example carries raw HTML (which the reader would read as
/// markup, rightly) or an image with an empty alt (which reads back as
/// `<img>` without an alt attribute either way, but differently).
fn outside_the_reader(markdown: &str) -> bool {
    let mut events: Vec<Event<'_>> = Vec::new();
    parse_into(markdown, Options::default(), &mut events);
    events
        .iter()
        .any(|event| matches!(event, Event::Html(_) | Event::InlineHtml(_)))
}

fn corpus(name: &str) -> (usize, usize, Vec<String>) {
    let mut checked = 0;
    let mut skipped = 0;
    let mut failures = Vec::new();
    for example in load(name) {
        if outside_the_reader(&example.markdown) {
            skipped += 1;
            continue;
        }
        checked += 1;
        let html = markdown_to_html(&example.markdown);
        let expected = normalized(&html);
        let actual = normalized(&html_to_html(&html));
        if actual != expected {
            failures.push(format!(
                "{name} example {} ({}):\n--- written ---\n{expected}\n--- read back ---\n{actual}\n",
                example.number, example.section
            ));
        }
    }
    (checked, skipped, failures)
}

#[test]
fn commonmark_html_reads_back_to_itself() {
    let (checked, skipped, failures) = corpus("commonmark.json");
    assert!(checked > 400, "checked {checked}, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ ({skipped} skipped):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn gfm_html_reads_back_to_itself() {
    let (checked, skipped, failures) = corpus("gfm.json");
    assert!(checked > 15, "checked {checked}, skipped {skipped}");
    assert!(
        failures.is_empty(),
        "{} of {checked} differ ({skipped} skipped):\n{}",
        failures.len(),
        failures.join("\n")
    );
}

const SOUP: &str = r#"<!DOCTYPE html>
<html><head><title>Ignored</title><style>p { color: red }</style>
<script>var x = "<p>not a paragraph</p>";</script></head>
<body>
<H1 class=title>Title &amp; more</H1>
<P>First paragraph
   with <B>bold <I>both</I></B> and <a href="/x?a=1&amp;b=2" title="t">a link</a>.
<p>Second, unclosed.
<ul>
  <li>one
  <li>two with <code>x &lt; y</code>
  <li><p>three</p>
</ul>
<div><span>Loose text</span> here, &copy; 2026.</div>
<table>
<tr><th align="right">A</th><th>B</th></tr>
<tr><td>1</td><td>2</td>
</table>
<hr/>
<blockquote>Quoted <em>text</em>.</blockquote>
<pre><code class="language-rs">fn main() {
    x &lt; 1
}
</code></pre>
<img src="pic.png" alt="A picture"><br>
Text after the break.
</body></html>
"#;

#[test]
fn tag_soup_reads_to_expected_markdown() {
    let markdown = html_to_markdown(SOUP);
    assert_eq!(
        markdown,
        "# Title \\& more\n\nFirst paragraph with **bold *both*** and [a link](/x?a=1\\&b=2 \"t\").\n\nSecond, unclosed.\n\n- one\n- two with `x < y`\n- three\n\nLoose text here, © 2026.\n\n| A | B |\n| --: | --- |\n| 1 | 2 |\n\n***\n\n> Quoted *text*.\n\n```rs\nfn main() {\n    x < 1\n}\n```\n\n![A picture](pic.png)\\\nText after the break.\n"
    );
}

#[test]
fn tag_soup_reads_to_text() {
    let text = html_to_text(SOUP);
    assert!(text.starts_with("Title & more\n"), "{text}");
    assert!(
        text.contains("First paragraph with bold both and a link (/x?a=1&b=2)."),
        "{text}"
    );
    assert!(
        !text.contains("not a paragraph"),
        "scripts are skipped: {text}"
    );
    assert!(!text.contains("color"), "styles are skipped: {text}");
}

#[test]
fn footnotes_and_task_lists_read_back() {
    let markdown = "A note[^1] and a task:\n\n- [x] done\n- [ ] open\n\n[^1]: The note.\n";
    let html = markdown_to_html(markdown);
    let read_back = html_to_markdown(&html);
    assert_eq!(
        read_back,
        "A note[^1] and a task:\n\n- [x] done\n- [ ] open\n\n[^1]: The note.\n"
    );
}

#[test]
fn entities_and_attributes_decode() {
    let html = "<p>Fish &amp; chips &lt;3 &#x2014; &#8212; &nbsp;done &unknown; &amp</p>";
    assert_eq!(
        html_to_markdown(html),
        "Fish \\& chips \\<3 — — \u{a0}done \\&unknown; \\&amp\n"
    );
    let html = "<p><a href='a&amp;b' title=\"it&#39;s\">x</a> <a href=unquoted>y</a></p>";
    assert_eq!(
        html_to_markdown(html),
        "[x](a\\&b \"it's\") [y](unquoted)\n"
    );
}

#[test]
fn loose_and_ordered_lists_keep_their_shape() {
    let html =
        "<ol start=\"3\">\n<li>\n<p>three</p>\n<p>more</p>\n</li>\n<li>\n<p>four</p>\n</li>\n</ol>";
    assert_eq!(html_to_markdown(html), "3. three\n\n   more\n\n4. four\n");
}
