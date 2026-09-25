//! CSV <-> JSON: generators, our pipelines, the `csv` + `serde_json`
//! reference pipelines, and reader-only modes for both sides.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converters::csv_to_json::{CSV_TO_JSON, TSV_TO_JSON};
use sublime::converters::json_to_csv::{JSON_TO_CSV, JSON_TO_TSV};

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-csv", [rows, path]) => generate_csv(rows, path),
        ("gen-json", [rows, path]) => generate_json(rows, path),
        ("ours-csv-json", [input, output]) => run_ours(&CSV_TO_JSON, input, output),
        ("ours-json-csv", [input, output]) => run_ours(&JSON_TO_CSV, input, output),
        ("crates-csv-json", [input, output]) => crates_csv_to_json(input, output),
        ("crates-json-csv", [input, output]) => crates_json_to_csv(input, output),
        ("ours-tsv-json", [input, output]) => run_ours(&TSV_TO_JSON, input, output),
        ("ours-json-tsv", [input, output]) => run_ours(&JSON_TO_TSV, input, output),
        ("crates-tsv-json", [input, output]) => crates_tsv_to_json(input, output),
        ("crates-json-tsv", [input, output]) => crates_json_to_tsv(input, output),
        ("ours-csv-read", [input]) => read_csv_only(input),
        ("crates-csv-read", [input]) => crates_read_csv_only(input),
        _ => Err(
            "csv-json modes: gen-csv <rows> <path> | gen-json <rows> <path> | ours-csv-json <in> <out> | ours-json-csv <in> <out> | crates-csv-json <in> <out> | crates-json-csv <in> <out> | ours-csv-read <in> | crates-csv-read <in>"
                .to_string(),
        ),
    }
}

fn generate_csv(rows: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(rows)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "id,name,email,city,score,note").map_err(|error| error.to_string())?;
    for index in 0..count {
        let note = if index % 7 == 0 {
            "\"quoted, with comma\""
        } else {
            "plain"
        };
        writeln!(
            writer,
            "{index},user{index},user{index}@example.com,City{},{},{note}",
            index % 1000,
            (index * 37) % 100
        )
        .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json(rows: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(rows)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    write!(writer, "[").map_err(|error| error.to_string())?;
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        write!(
            writer,
            "{{\"id\":\"{index}\",\"name\":\"user{index}\",\"email\":\"user{index}@example.com\",\"city\":\"City{}\",\"score\":\"{}\",\"note\":\"plain\"}}",
            index % 1000,
            (index * 37) % 100
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "]").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

/// Reads every record with our CSV reader and discards it. Isolates reader cost.
fn read_csv_only(input: &str) -> Result<(), String> {
    use sublime::io::csv::{CsvReader, Record};
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = CsvReader::new(file);
    let mut record = Record::new();
    let mut field_total: u64 = 0;
    loop {
        let has_record = reader
            .read_record(&mut record)
            .map_err(|error| error.to_string())?;
        if !has_record {
            break;
        }
        field_total += record.len() as u64;
    }
    eprintln!("fields: {field_total}");
    Ok(())
}

/// Reads every record with the `csv` crate and discards it.
fn crates_read_csv_only(input: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = csv::Reader::from_reader(BufReader::new(file));
    let mut record = csv::StringRecord::new();
    let mut field_total: u64 = 0;
    while reader
        .read_record(&mut record)
        .map_err(|error| error.to_string())?
    {
        field_total += record.len() as u64;
    }
    eprintln!("fields: {field_total}");
    Ok(())
}

fn crates_csv_to_json(input: &str, output: &str) -> Result<(), String> {
    use serde::ser::{SerializeSeq, Serializer};
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = csv::Reader::from_reader(BufReader::new(file));
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let output_file = File::create(output).map_err(|error| error.to_string())?;
    let writer = BufWriter::new(output_file);
    let mut serializer = serde_json::Serializer::new(writer);
    let mut sequence = serializer
        .serialize_seq(None)
        .map_err(|error| error.to_string())?;
    let mut record = csv::StringRecord::new();
    while reader
        .read_record(&mut record)
        .map_err(|error| error.to_string())?
    {
        let object = RecordObject {
            headers: &headers,
            record: &record,
        };
        sequence
            .serialize_element(&object)
            .map_err(|error| error.to_string())?;
    }
    sequence.end().map_err(|error| error.to_string())?;
    let mut writer = serializer.into_inner();
    writer.flush().map_err(|error| error.to_string())
}

struct RecordObject<'a> {
    headers: &'a csv::StringRecord,
    record: &'a csv::StringRecord,
}

impl serde::Serialize for RecordObject<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        use serde::ser::SerializeMap;
        let mut map = serializer.serialize_map(Some(self.record.len()))?;
        for (key, value) in self.headers.iter().zip(self.record.iter()) {
            map.serialize_entry(key, value)?;
        }
        map.end()
    }
}

/// Reference pipeline: `serde_json` stream of objects -> `csv` writer. Two passes like ours.
fn crates_json_to_csv(input: &str, output: &str) -> Result<(), String> {
    use serde_json::Value;
    let first_file = File::open(input).map_err(|error| error.to_string())?;
    let first: Vec<Value> =
        serde_json::from_reader(BufReader::new(first_file)).map_err(|error| error.to_string())?;
    let mut keys: Vec<String> = Vec::new();
    for value in &first {
        if let Value::Object(map) = value {
            for key in map.keys() {
                if !keys.contains(key) {
                    keys.push(key.clone());
                }
            }
        }
    }
    let output_file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = csv::Writer::from_writer(BufWriter::new(output_file));
    writer
        .write_record(&keys)
        .map_err(|error| error.to_string())?;
    for value in &first {
        if let Value::Object(map) = value {
            let row: Vec<String> = keys
                .iter()
                .map(|key| match map.get(key) {
                    Some(Value::String(text)) => text.clone(),
                    Some(Value::Null) | None => String::new(),
                    Some(other) => other.to_string(),
                })
                .collect();
            writer
                .write_record(&row)
                .map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())
}

/// The `csv` crate with a tab delimiter into `serde_json`, as `crates_csv_to_json`.
fn crates_tsv_to_json(input: &str, output: &str) -> Result<(), String> {
    use serde::ser::{SerializeSeq, Serializer};
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(b'\t')
        .from_reader(BufReader::new(file));
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut serializer = serde_json::Serializer::new(BufWriter::new(out));
    let mut sequence = serializer.serialize_seq(None).map_err(|error| error.to_string())?;
    let mut record = csv::StringRecord::new();
    while reader.read_record(&mut record).map_err(|error| error.to_string())? {
        let map: std::collections::BTreeMap<&str, &str> = headers.iter().zip(record.iter()).collect();
        sequence.serialize_element(&map).map_err(|error| error.to_string())?;
    }
    sequence.end().map_err(|error| error.to_string())?;
    Ok(())
}

/// `serde_json` into the `csv` crate's writer with a tab delimiter.
fn crates_json_to_tsv(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let rows: Vec<serde_json::Map<String, serde_json::Value>> =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let out = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = csv::WriterBuilder::new().delimiter(b'\t').from_writer(BufWriter::new(out));
    let mut keys: Vec<String> = Vec::new();
    for row in &rows {
        for key in row.keys() {
            if !keys.contains(key) {
                keys.push(key.clone());
            }
        }
    }
    writer.write_record(&keys).map_err(|error| error.to_string())?;
    for row in &rows {
        let cells: Vec<String> = keys
            .iter()
            .map(|key| match row.get(key) {
                Some(serde_json::Value::String(text)) => text.clone(),
                Some(serde_json::Value::Null) | None => String::new(),
                Some(other) => other.to_string(),
            })
            .collect();
        writer.write_record(&cells).map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}
