//! YAML <-> JSON: generators for two shapes, our pipelines, and `serde_yaml`
//! with `serde_json` as the reference pipelines.
//!
//! Dense: a sequence of mappings, each a record of short keys and mixed
//! scalars (integers, floats, booleans, quoted and plain strings, flow
//! sequences, nested mappings). Prose: one mapping per unit holding a
//! literal block scalar paragraph.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::json_to_yaml::JsonToYaml;
use sublime::converters::yaml_to_json::YamlToJson;

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-yaml", [units, path]) => generate_yaml(units, path),
        ("gen-yaml-prose", [units, path]) => generate_yaml_prose(units, path),
        ("gen-json", [units, path]) => generate_json(units, path),
        ("gen-json-prose", [units, path]) => generate_json_prose(units, path),
        ("ours-yaml-json", [input, output]) => run_ours(&YamlToJson, input, output),
        ("ours-json-yaml", [input, output]) => run_ours(&JsonToYaml, input, output),
        ("crates-yaml-json", [input, output]) => crates_yaml_to_json(input, output),
        ("crates-json-yaml", [input, output]) => crates_json_to_yaml(input, output),
        _ => Err(
            "yaml-json modes: gen-yaml <units> <path> | gen-yaml-prose <units> <path> | gen-json <units> <path> | gen-json-prose <units> <path> | ours-yaml-json <in> <out> | ours-json-yaml <in> <out> | crates-yaml-json <in> <out> | crates-json-yaml <in> <out>"
                .to_string(),
        ),
    }
}

const WORDS: [&str; 12] = [
    "converter", "lossless", "fidelity", "planner", "arena", "buffer", "table", "record",
    "stream", "package", "schema", "document",
];

/// Sentences as lines, so a paragraph is a block scalar of several lines.
fn paragraph_lines(index: u64, out: &mut Vec<String>) {
    let sentences = 4 + (index % 5) as usize;
    for sentence in 0..sentences {
        let words = 8 + ((index + sentence as u64) % 9) as usize;
        let mut line = String::new();
        for word in 0..words {
            let pick = WORDS[((index + sentence as u64 * 7 + word as u64 * 3) % 12) as usize];
            if word == 0 {
                let mut chars = pick.chars();
                if let Some(first) = chars.next() {
                    line.push(first.to_ascii_uppercase());
                    line.push_str(chars.as_str());
                }
            } else {
                line.push(' ');
                line.push_str(pick);
            }
        }
        line.push('.');
        out.push(line);
    }
}

fn open_output(path: &str) -> Result<BufWriter<File>, String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    Ok(BufWriter::new(file))
}

fn generate_yaml(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    writeln!(writer, "title: benchmark\nversion: 3\nrecords:").map_err(|error| error.to_string())?;
    for index in 0..count {
        let quoted = if index % 7 == 0 { "\"with \\\"quotes\\\" and a \\\\ backslash\"" } else { "plain" };
        writeln!(
            writer,
            "  - id: {index}\n    name: user{index}\n    email: user{index}@example.com\n    score: {}.{}\n    active: {}\n    created: 2026-09-{:02}T{:02}:00:00Z\n    tags: [a, b{}, c]\n    note: {quoted}\n    limits:\n      min: {}\n      max: {}",
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

fn generate_yaml_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    writeln!(writer, "sections:").map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    for index in 0..count {
        lines.clear();
        paragraph_lines(index, &mut lines);
        writeln!(writer, "  s{index}:\n    heading: Section {index}\n    body: |").map_err(|error| error.to_string())?;
        for line in &lines {
            writeln!(writer, "      {line}").map_err(|error| error.to_string())?;
        }
        writeln!(writer, "    words: {}", lines.iter().map(|line| line.split(' ').count()).sum::<usize>())
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    write!(writer, "{{\"title\":\"benchmark\",\"version\":3,\"records\":[").map_err(|error| error.to_string())?;
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
    write!(writer, "{{\"sections\":{{").map_err(|error| error.to_string())?;
    let mut lines = Vec::new();
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        lines.clear();
        paragraph_lines(index, &mut lines);
        let body = lines.join("\\n");
        write!(
            writer,
            "\"s{index}\":{{\"heading\":\"Section {index}\",\"body\":\"{body}\\n\",\"words\":{}}}",
            lines.iter().map(|line| line.split(' ').count()).sum::<usize>()
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "}}}}").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn crates_yaml_to_json(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let value: serde_yaml::Value =
        serde_yaml::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let mut writer = open_output(output)?;
    serde_json::to_writer(&mut writer, &value).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn crates_json_to_yaml(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let mut writer = open_output(output)?;
    serde_yaml::to_writer(&mut writer, &value).map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
