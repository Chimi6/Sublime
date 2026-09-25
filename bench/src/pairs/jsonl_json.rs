//! JSON Lines <-> JSON: the generator writes the csv-json records one per
//! line; ours streams token to token; the reference is `serde_json`'s
//! stream deserializer into a `Vec` and back out.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::rows::{JsonToJsonl, JsonlToJson};

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-jsonl", [rows, path]) => generate_jsonl(rows, path),
        ("ours-jsonl-json", [input, output]) => run_ours(&JsonlToJson, input, output),
        ("ours-json-jsonl", [input, output]) => run_ours(&JsonToJsonl, input, output),
        ("crates-jsonl-json", [input, output]) => crates_jsonl_to_json(input, output),
        ("crates-json-jsonl", [input, output]) => crates_json_to_jsonl(input, output),
        _ => Err(
            "jsonl-json modes: gen-jsonl <rows> <path> | ours-jsonl-json <in> <out> | ours-json-jsonl <in> <out> | crates-jsonl-json <in> <out> | crates-json-jsonl <in> <out>"
                .to_string(),
        ),
    }
}

fn generate_jsonl(rows: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(rows)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for index in 0..count {
        writeln!(
            writer,
            "{{\"id\":{index},\"name\":\"user{index}\",\"email\":\"user{index}@example.com\",\"city\":\"City{}\",\"score\":{},\"tags\":[\"a\",\"b\"],\"note\":\"plain\"}}",
            index % 1000,
            (index * 37) % 100
        )
        .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn crates_jsonl_to_json(input: &str, output: &str) -> Result<(), String> {
    use serde::ser::{SerializeSeq, Serializer};
    let file = File::open(input).map_err(|error| error.to_string())?;
    let stream = serde_json::Deserializer::from_reader(BufReader::new(file)).into_iter::<serde_json::Value>();
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut serializer = serde_json::Serializer::new(BufWriter::new(out));
    let mut sequence = serializer.serialize_seq(None).map_err(|error| error.to_string())?;
    for value in stream {
        let value = value.map_err(|error| error.to_string())?;
        sequence.serialize_element(&value).map_err(|error| error.to_string())?;
    }
    sequence.end().map_err(|error| error.to_string())?;
    Ok(())
}

fn crates_json_to_jsonl(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let values: Vec<serde_json::Value> = serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(out);
    for value in &values {
        serde_json::to_writer(&mut writer, value).map_err(|error| error.to_string())?;
        writer.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}
