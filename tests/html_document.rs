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

/// A space before an inline element's end still separates words, as a
/// browser shows it.
#[test]
fn a_space_inside_an_ending_element_is_kept() {
    assert_eq!(
        html_to_markdown("<p>Regular, <b>bold, </b><i>italic</i> text.</p>"),
        "Regular, **bold,** *italic* text.\n"
    );
}

/// EPUB's and pandoc's notes: references by role or `epub:type`, notes in
/// a section of their own or standing alone, links back left out.
#[test]
fn epub_and_pandoc_notes_are_footnotes() {
    let markdown = html_to_markdown(concat!(
        "<p>One<a href=\"#fn1\" epub:type=\"noteref\">1</a> and two",
        "<a href=\"notes.xhtml#n2\" role=\"doc-noteref\">2</a>.</p>",
        "<section epub:type=\"footnotes\"><h2>Notes</h2><hr/>",
        "<aside id=\"fn1\" epub:type=\"footnote\"><p>First. <a href=\"#r1\" role=\"doc-backlink\">back</a></p></aside>",
        "</section>",
        "<ol><li id=\"n2\" epub:type=\"endnote\"><p>Second.</p></li></ol>",
    ));
    assert_eq!(
        markdown,
        "One[^fn1] and two[^n2].\n\n[^fn1]: First.\n\n[^n2]: Second.\n"
    );
}

/// Merged cells hold their place: a cell spanning columns or rows leaves
/// empty cells where it covers the grid, so the cells beside it line up.
#[test]
fn spanned_cells_keep_the_grid() {
    let markdown = html_to_markdown(concat!(
        "<table><tr><th colspan=\"2\">Wide</th><th>C</th></tr>",
        "<tr><td rowspan=\"2\">Tall</td><td>b2</td><td>c2</td></tr>",
        "<tr><td>b3</td><td>c3</td></tr></table>",
    ));
    assert_eq!(
        markdown,
        "| Wide |  | C |\n| --- | --- | --- |\n| Tall | b2 | c2 |\n|  | b3 | c3 |\n"
    );
}

/// A picture framed in an SVG (an e-book's cover page) is the picture.
#[test]
fn a_picture_in_an_svg_frame() {
    let markdown = html_to_markdown(concat!(
        "<div><svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 10 10\">",
        "<image width=\"10\" height=\"10\" xlink:href=\"cover.jpg\"/></svg></div>",
    ));
    assert_eq!(markdown, "![](cover.jpg)\n");
}

/// Project Gutenberg's notes: a `sup.footnoteref` reference, and blocks
/// with ids in a `div.footnotes`, each led by a link back.
#[test]
fn gutenberg_notes_are_footnotes() {
    let markdown = html_to_markdown(concat!(
        "<p>A line.<sup class=\"footnoteref\"><a href=\"p1.xhtml#FNDEF.1\" id=\"FNREF.1\" class=\"pginternal\">1</a></sup></p>",
        "<div class=\"footnotes\"><div class=\"footnote\" id=\"FNDEF.1\">",
        "<a href=\"p1.xhtml#FNREF.1\" class=\"pginternal\">[1]</a> The note.</div></div>",
    ));
    assert_eq!(markdown, "A line.[^FNDEF.1]\n\n[^FNDEF.1]: The note.\n");
}
