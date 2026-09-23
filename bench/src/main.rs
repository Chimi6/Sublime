//! Comparison harness. Runs our converters and the conventional-crate
//! pipelines on the same inputs. Never part of the shipped binary.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use sublime::converter::{ConvertOptions, Converter, Input};
use sublime::converters::csv_to_json::CsvToJson;
use sublime::converters::json_to_csv::JsonToCsv;
use sublime::event::{Context, NullSink};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, rows, path] if command == "gen-csv" => generate_csv(rows, path),
        [command, rows, path] if command == "gen-json" => generate_json(rows, path),
        [command, input, output] if command == "ours-csv-json" => run_ours(&CsvToJson, input, output),
        [command, input, output] if command == "ours-json-csv" => run_ours(&JsonToCsv, input, output),
        [command, input, output] if command == "crates-csv-json" => crates_csv_to_json(input, output),
        [command, input, output] if command == "crates-json-csv" => crates_json_to_csv(input, output),
        _ => {
            eprintln!("usage: bench <gen-csv|gen-json> <rows> <path> | <ours-csv-json|ours-json-csv|crates-csv-json|crates-json-csv> <in> <out>");
            std::process::exit(4);
        }
    };
    if let Err(error) = result {
        eprintln!("bench: {error}");
        std::process::exit(1);
    }
}

fn parse_rows(rows: &str) -> Result<u64, String> {
    rows.parse::<u64>().map_err(|_| format!("bad row count '{rows}'"))
}

fn generate_csv(rows: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(rows)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writeln!(writer, "id,name,email,city,score,note").map_err(|error| error.to_string())?;
    for index in 0..count {
        let note = if index % 7 == 0 { "\"quoted, with comma\"" } else { "plain" };
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

fn run_ours(converter: &dyn Converter, input: &str, output: &str) -> Result<(), String> {
    let mut input_file = File::open(input).map_err(|error| error.to_string())?;
    let output_file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(output_file);
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    converter
        .convert(Input::Rewindable(&mut input_file), &mut writer, &mut context)
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

/// Reference pipeline: `csv` crate records -> `serde_json` map serializer, streaming.
fn crates_csv_to_json(input: &str, output: &str) -> Result<(), String> {
    use serde::ser::{SerializeSeq, Serializer};
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = csv::Reader::from_reader(BufReader::new(file));
    let headers = reader.headers().map_err(|error| error.to_string())?.clone();
    let output_file = File::create(output).map_err(|error| error.to_string())?;
    let writer = BufWriter::new(output_file);
    let mut serializer = serde_json::Serializer::new(writer);
    let mut sequence = serializer.serialize_seq(None).map_err(|error| error.to_string())?;
    let mut record = csv::StringRecord::new();
    while reader.read_record(&mut record).map_err(|error| error.to_string())? {
        let object = RecordObject { headers: &headers, record: &record };
        sequence.serialize_element(&object).map_err(|error| error.to_string())?;
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
    let first: Vec<Value> = serde_json::from_reader(BufReader::new(first_file)).map_err(|error| error.to_string())?;
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
    writer.write_record(&keys).map_err(|error| error.to_string())?;
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
            writer.write_record(&row).map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())
}
