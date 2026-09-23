//! Helpers shared by every pair.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converter::{ConvertOptions, Converter, Input};
use sublime::event::{Context, NullSink};

pub fn parse_rows(rows: &str) -> Result<u64, String> {
    rows.parse::<u64>()
        .map_err(|_| format!("bad row count '{rows}'"))
}

/// Runs one of our converters from a file to a file with a silent sink.
pub fn run_ours(converter: &dyn Converter, input: &str, output: &str) -> Result<(), String> {
    let mut input_file = File::open(input).map_err(|error| error.to_string())?;
    let output_file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(output_file);
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    converter
        .convert(
            Input::Rewindable(&mut input_file),
            &mut writer,
            &mut context,
        )
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
