//! Markdown -> Word: our pipeline (parser, events bridge, Word writer) and
//! a reference pipeline of `pulldown-cmark` feeding `docx-rs`, which
//! writes paragraphs and runs (headings by size, bold and italic, code as
//! text) and nothing else. Inputs come from the markdown-html generator.

use std::fs::File;
use std::io::BufWriter;

use docx_rs::{Docx, Paragraph, Run};
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use sublime::converters::markdown_to_docx::MarkdownToDocx;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&MarkdownToDocx, input, output),
        ("crates", [input, output]) => crates_markdown_to_docx(input, output),
        _ => Err("markdown-docx modes: ours <in> <out> | crates <in> <out>".to_string()),
    }
}

fn crates_markdown_to_docx(input: &str, output: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    let mut docx = Docx::new();
    let mut paragraph = Paragraph::new();
    let mut bold = 0u32;
    let mut italic = 0u32;
    let mut heading: Option<u8> = None;
    let mut open = false;
    for event in Parser::new_ext(&text, options) {
        match event {
            Event::Start(Tag::Paragraph) | Event::Start(Tag::Item) | Event::Start(Tag::CodeBlock(_)) => {
                open = true;
            }
            Event::Start(Tag::Heading { level, .. }) => {
                heading = Some(level as u8);
                open = true;
            }
            Event::Start(Tag::Strong) => bold += 1,
            Event::End(TagEnd::Strong) => bold = bold.saturating_sub(1),
            Event::Start(Tag::Emphasis) => italic += 1,
            Event::End(TagEnd::Emphasis) => italic = italic.saturating_sub(1),
            Event::Text(piece) | Event::Code(piece) => {
                let mut run = Run::new().add_text(piece.as_ref());
                if bold > 0 {
                    run = run.bold();
                }
                if italic > 0 {
                    run = run.italic();
                }
                if let Some(level) = heading {
                    run = run.size(usize::from(40 - level * 4));
                }
                paragraph = paragraph.add_run(run);
            }
            Event::SoftBreak => paragraph = paragraph.add_run(Run::new().add_text(" ")),
            Event::End(TagEnd::Paragraph)
            | Event::End(TagEnd::Heading(_))
            | Event::End(TagEnd::Item)
            | Event::End(TagEnd::CodeBlock)
            | Event::End(TagEnd::TableRow)
            | Event::End(TagEnd::TableHead) => {
                if open {
                    docx = docx.add_paragraph(std::mem::replace(&mut paragraph, Paragraph::new()));
                    open = false;
                    heading = None;
                }
            }
            _ => {}
        }
    }
    let file = File::create(output).map_err(|error| error.to_string())?;
    docx.build()
        .pack(BufWriter::new(file))
        .map_err(|error| error.to_string())
}
