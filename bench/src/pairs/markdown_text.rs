//! Markdown -> plain text. Ours renders structure (list markers, aligned
//! tables, numbered footnotes); the reference is the least work a
//! `pulldown-cmark` user could do: concatenate the text of every event with
//! a newline at each block end. It is a floor, not a peer.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::markdown_to_text::MarkdownToText;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&MarkdownToText, input, output),
        ("crates", [input, output]) => crates_markdown_to_text(input, output),
        _ => Err("markdown-text modes: ours <in> <out> | crates <in> <out>".to_string()),
    }
}

fn crates_markdown_to_text(input: &str, output: &str) -> Result<(), String> {
    use pulldown_cmark::{Event, Options, Parser, TagEnd};
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    let mut rendered = String::with_capacity(text.len());
    for event in Parser::new_ext(&text, options) {
        match event {
            Event::Text(text) | Event::Code(text) => rendered.push_str(&text),
            Event::SoftBreak | Event::HardBreak => rendered.push('\n'),
            Event::End(TagEnd::Paragraph | TagEnd::Heading(_) | TagEnd::Item) => {
                rendered.push('\n');
            }
            _ => {}
        }
    }
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(rendered.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
