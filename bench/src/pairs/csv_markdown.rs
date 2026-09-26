//! CSV <-> Markdown table: ours against the conventional way, the `csv`
//! crate with a hand-written table printer (CSV to Markdown) and
//! `pulldown-cmark`'s events into the `csv` crate (Markdown to CSV).

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::rows_document::{CSV_TO_MARKDOWN, MARKDOWN_TO_CSV};

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-csv-markdown", [input, output]) => run_ours(&CSV_TO_MARKDOWN, input, output),
        ("ours-markdown-csv", [input, output]) => run_ours(&MARKDOWN_TO_CSV, input, output),
        ("crates-csv-markdown", [input, output]) => crates_csv_to_markdown(input, output),
        ("crates-markdown-csv", [input, output]) => crates_markdown_to_csv(input, output),
        _ => Err(
            "csv-markdown modes: ours-csv-markdown <in> <out> | ours-markdown-csv <in> <out> | crates-csv-markdown <in> <out> | crates-markdown-csv <in> <out>"
                .to_string(),
        ),
    }
}

/// A cell as GFM reads it back unchanged: pipes, emphasis, code, link,
/// autolink, and entity characters escaped, line breaks folded. The same
/// job ours does; a printer that escaped only pipes would turn every
/// email and URL in the table into a link.
fn push_cell(out: &mut String, cell: &str) {
    let bytes = cell.as_bytes();
    for (index, character) in cell.char_indices() {
        match character {
            '\n' | '\r' => out.push(' '),
            '\\' | '*' | '_' | '`' | '[' | ']' | '<' | '~' | '&' | '@' | '|' => {
                out.push('\\');
                out.push(character);
            }
            ':' if bytes[index + 1..].starts_with(b"//") => out.push_str("\\:"),
            '.' if index >= 3 && bytes[index - 3..index].eq_ignore_ascii_case(b"www") => {
                out.push_str("\\.")
            }
            other => out.push(other),
        }
    }
}

fn crates_csv_to_markdown(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .from_reader(BufReader::new(file));
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(out);
    let mut record = csv::StringRecord::new();
    let mut line = String::new();
    let mut width = 0;
    let mut first = true;
    while reader
        .read_record(&mut record)
        .map_err(|error| error.to_string())?
    {
        line.clear();
        if first {
            width = record.len();
        }
        line.push('|');
        for index in 0..width {
            line.push(' ');
            push_cell(&mut line, record.get(index).unwrap_or(""));
            line.push_str(" |");
        }
        line.push('\n');
        if first {
            line.push('|');
            for _ in 0..width {
                line.push_str(" --- |");
            }
            line.push('\n');
            first = false;
        }
        writer
            .write_all(line.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn crates_markdown_to_csv(input: &str, output: &str) -> Result<(), String> {
    use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(out));
    let mut cells: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_table = false;
    let mut done = false;
    // The same extensions our parser always runs (see markdown_json.rs).
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    for event in Parser::new_ext(&text, options) {
        if done {
            break;
        }
        match event {
            Event::Start(Tag::Table(_)) => in_table = true,
            Event::End(TagEnd::Table) => done = true,
            _ if !in_table => {}
            Event::Start(Tag::TableCell) => current.clear(),
            Event::End(TagEnd::TableCell) => cells.push(std::mem::take(&mut current)),
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => {
                writer
                    .write_record(&cells)
                    .map_err(|error| error.to_string())?;
                cells.clear();
            }
            Event::Text(text) | Event::Code(text) => current.push_str(&text),
            Event::SoftBreak | Event::HardBreak => current.push(' '),
            _ => {}
        }
    }
    writer.flush().map_err(|error| error.to_string())
}
