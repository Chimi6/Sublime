//! TOML <-> JSON: generators for two shapes, our pipelines, and the `toml`
//! crate with `serde_json` as the reference pipelines.
//!
//! Dense: an array of tables, each a record of short keys and mixed
//! scalars (integers, floats, booleans, dates, inline arrays). Prose: one
//! header table per unit holding a long multi-line string paragraph.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::json_to_toml::JsonToToml;
use sublime::converters::toml_to_json::TomlToJson;

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-toml", [units, path]) => generate_toml(units, path),
        ("gen-toml-prose", [units, path]) => generate_toml_prose(units, path),
        ("gen-json", [units, path]) => generate_json(units, path),
        ("gen-json-prose", [units, path]) => generate_json_prose(units, path),
        ("ours-toml-json", [input, output]) => run_ours(&TomlToJson, input, output),
        ("ours-json-toml", [input, output]) => run_ours(&JsonToToml, input, output),
        ("crates-toml-json", [input, output]) => crates_toml_to_json(input, output),
        ("crates-json-toml", [input, output]) => crates_json_to_toml(input, output),
        _ => Err(
            "toml-json modes: gen-toml <units> <path> | gen-toml-prose <units> <path> | gen-json <units> <path> | gen-json-prose <units> <path> | ours-toml-json <in> <out> | ours-json-toml <in> <out> | crates-toml-json <in> <out> | crates-json-toml <in> <out>"
                .to_string(),
        ),
    }
}

const WORDS: [&str; 12] = [
    "converter", "lossless", "fidelity", "planner", "arena", "buffer", "table", "record",
    "stream", "package", "schema", "document",
];

fn paragraph(index: u64, out: &mut String) {
    let sentences = 4 + (index % 5) as usize;
    for sentence in 0..sentences {
        let words = 8 + ((index + sentence as u64) % 9) as usize;
        for word in 0..words {
            let pick = WORDS[((index + sentence as u64 * 7 + word as u64 * 3) % 12) as usize];
            if word == 0 {
                let mut chars = pick.chars();
                if let Some(first) = chars.next() {
                    out.push(first.to_ascii_uppercase());
                    out.push_str(chars.as_str());
                }
            } else {
                out.push(' ');
                out.push_str(pick);
            }
        }
        out.push_str(". ");
    }
}

fn open_output(path: &str) -> Result<BufWriter<File>, String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    Ok(BufWriter::new(file))
}

fn generate_toml(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    writeln!(writer, "title = \"benchmark\"\nversion = 3\n").map_err(|error| error.to_string())?;
    for index in 0..count {
        let quoted = if index % 7 == 0 { "with \\\"quotes\\\" and a \\\\ backslash" } else { "plain" };
        writeln!(
            writer,
            "[[record]]\nid = {index}\nname = \"user{index}\"\nemail = \"user{index}@example.com\"\nscore = {}.{}\nactive = {}\ncreated = 2026-09-{:02}T{:02}:00:00Z\ntags = [\"a\", \"b{}\", \"c\"]\nnote = \"{quoted}\"\nlimits = {{ min = {}, max = {} }}\n",
            (index * 37) % 100,
            index % 10,
            index % 3 == 0,
            1 + index % 28,
            index % 24,
            index % 5,
            index % 50,
            50 + index % 50,
        )
        .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn generate_toml_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    let mut text = String::new();
    for index in 0..count {
        text.clear();
        paragraph(index, &mut text);
        writeln!(
            writer,
            "[section.s{index}]\nheading = \"Section {index}\"\nbody = \"\"\"\n{text}\n\"\"\"\nwords = {}\n",
            text.split(' ').count()
        )
        .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    write!(writer, "{{\"title\":\"benchmark\",\"version\":3,\"record\":[").map_err(|error| error.to_string())?;
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        let quoted = if index % 7 == 0 { "with \\\"quotes\\\" and a \\\\ backslash" } else { "plain" };
        write!(
            writer,
            "{{\"id\":{index},\"name\":\"user{index}\",\"email\":\"user{index}@example.com\",\"score\":{}.{},\"active\":{},\"created\":\"2026-09-{:02}T{:02}:00:00Z\",\"tags\":[\"a\",\"b{}\",\"c\"],\"note\":\"{quoted}\",\"limits\":{{\"min\":{},\"max\":{}}}}}",
            (index * 37) % 100,
            index % 10,
            index % 3 == 0,
            1 + index % 28,
            index % 24,
            index % 5,
            index % 50,
            50 + index % 50,
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "]}}").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    write!(writer, "{{\"section\":{{").map_err(|error| error.to_string())?;
    let mut text = String::new();
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        text.clear();
        paragraph(index, &mut text);
        write!(
            writer,
            "\"s{index}\":{{\"heading\":\"Section {index}\",\"body\":\"{text}\\n\",\"words\":{}}}",
            text.split(' ').count()
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "}}}}").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn crates_toml_to_json(input: &str, output: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let table: toml::Table = toml::from_str(&text).map_err(|error| error.to_string())?;
    let mut writer = open_output(output)?;
    serde_json::to_writer(&mut writer, &table).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn crates_json_to_toml(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let table: toml::Table =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let text = toml::to_string(&table).map_err(|error| error.to_string())?;
    let mut writer = open_output(output)?;
    writer.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
