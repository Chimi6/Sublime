//! Markdown <-> the event stream as JSON, and the Markdown writer.
//!
//! - `ours <in> <out>` / `crates <in> <out>`: Markdown -> JSON. The
//!   reference serializes `pulldown-cmark` events with `serde_json`.
//! - `ours-back <in.json> <out.md>`: our JSON -> Markdown converter.
//! - `crates-back <in.json> <out.md>`: `serde_json` deserializing the
//!   events `crates` wrote, rendered by `pulldown-cmark-to-cmark`.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::markdown_json_to_markdown::MarkdownJsonToMarkdown;
use sublime::converters::markdown_to_json::MarkdownToJson;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&MarkdownToJson, input, output),
        ("crates", [input, output]) => crates_markdown_to_json(input, output),
        ("ours-back", [input, output]) => run_ours(&MarkdownJsonToMarkdown, input, output),
        ("crates-back", [input, output]) => crates_markdown_to_markdown(input, output),
        _ => Err(
            "markdown-json modes: ours <in> <out> | crates <in> <out> | ours-back <in> <out> | crates-back <in> <out>"
                .to_string(),
        ),
    }
}

fn parser_options() -> pulldown_cmark::Options {
    use pulldown_cmark::Options;
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    options
}

fn crates_markdown_to_json(input: &str, output: &str) -> Result<(), String> {
    use pulldown_cmark::Parser;
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer.write_all(b"[").map_err(|error| error.to_string())?;
    let mut first = true;
    for event in Parser::new_ext(&text, parser_options()) {
        if !first {
            writer.write_all(b",").map_err(|error| error.to_string())?;
        }
        first = false;
        serde_json::to_writer(&mut writer, &event).map_err(|error| error.to_string())?;
    }
    writer.write_all(b"]").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

/// Deserializes the events `crates` wrote and renders them with
/// `pulldown-cmark-to-cmark`; the whole event vector is held in memory, as
/// `serde_json` needs for an array.
fn crates_markdown_to_markdown(input: &str, output: &str) -> Result<(), String> {
    use pulldown_cmark::Event;
    let json = std::fs::read(input).map_err(|error| error.to_string())?;
    let events: Vec<Event<'_>> =
        serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    let mut rendered = String::with_capacity(json.len() / 2);
    pulldown_cmark_to_cmark::cmark(events.into_iter(), &mut rendered)
        .map_err(|error| error.to_string())?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(rendered.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
