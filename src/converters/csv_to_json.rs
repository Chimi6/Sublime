//! CSV -> JSON. Header row becomes keys; every value is a string.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, Record};
use crate::io::json::JsonWriter;

const NAME: &str = "csv-to-json";
const PROGRESS_INTERVAL: u64 = 4096;

pub struct CsvToJson;

impl Converter for CsvToJson {
    fn name(&self) -> &'static str {
        NAME
    }

    fn from(&self) -> &'static Format {
        &formats::CSV
    }

    fn to(&self) -> &'static Format {
        &formats::JSON
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Lossless
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let mut reader = CsvReader::new(input);
        let mut writer = JsonWriter::new(output);
        let mut record = Record::new();

        writer.begin_array()?;
        let has_header = reader.read_record(&mut record)?;
        if !has_header {
            writer.end_array()?;
            return Ok(());
        }
        let keys: Vec<String> = record.fields().map(str::to_string).collect();

        let mut record_count: u64 = 0;
        loop {
            let has_record = reader.read_record(&mut record)?;
            if !has_record {
                break;
            }
            record_count += 1;
            write_object(&keys, &record, &mut writer, context)?;
            if record_count % PROGRESS_INTERVAL == 0 {
                context.progress(NAME, reader.bytes_consumed());
            }
        }
        writer.end_array()?;
        writer.flush()?;
        Ok(())
    }
}

fn write_object<W: Write>(
    keys: &[String],
    record: &Record,
    writer: &mut JsonWriter<W>,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let key_count = keys.len();
    let field_count = record.len();
    let common_count = key_count.min(field_count);

    writer.begin_object()?;
    for (index, key) in keys.iter().enumerate().take(common_count) {
        let value = record.field(index).unwrap_or("");
        writer.key(key)?;
        writer.string(value)?;
    }
    writer.end_object()?;

    if field_count < key_count {
        let missing = key_count - field_count;
        let message = format!(
            "line {}: row has {missing} fewer field(s) than the header; missing keys omitted",
            record.line()
        );
        context.warning(message);
    }
    if field_count > key_count {
        let extra = field_count - key_count;
        let location = Location {
            line: record.line(),
            column: key_count as u64 + 1,
        };
        let description =
            format!("row has {extra} more field(s) than the header; extra fields dropped");
        context.loss(NAME, location, description);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::ConvertOptions;
    use crate::event::{CollectingSink, Event};

    fn convert(input: &[u8]) -> (String, CollectingSink) {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        {
            let mut context = Context::new(&mut sink, &options);
            let mut source: &[u8] = input;
            let converter = CsvToJson;
            converter
                .convert(Input::Stream(&mut source), &mut output, &mut context)
                .unwrap();
        }
        (String::from_utf8(output).unwrap(), sink)
    }

    #[test]
    fn converts_rows_to_objects() {
        let (json, sink) = convert(b"name,age\nAda,36\nLin,29\n");
        assert_eq!(
            json,
            "[{\"name\":\"Ada\",\"age\":\"36\"},{\"name\":\"Lin\",\"age\":\"29\"}]"
        );
        assert!(sink.report().is_lossless());
    }

    #[test]
    fn empty_input_is_an_empty_array() {
        let (json, _) = convert(b"");
        assert_eq!(json, "[]");
    }

    #[test]
    fn header_only_is_an_empty_array() {
        let (json, _) = convert(b"a,b\n");
        assert_eq!(json, "[]");
    }

    #[test]
    fn short_row_omits_missing_keys_and_warns() {
        let (json, sink) = convert(b"a,b,c\n1,2\n");
        assert_eq!(json, "[{\"a\":\"1\",\"b\":\"2\"}]");
        let has_warning = sink
            .events()
            .iter()
            .any(|event| matches!(event, Event::Warning { .. }));
        assert!(has_warning);
        assert!(sink.report().is_lossless());
    }

    #[test]
    fn long_row_drops_extras_and_reports_loss() {
        let (json, sink) = convert(b"a,b\n1,2,3,4\n");
        assert_eq!(json, "[{\"a\":\"1\",\"b\":\"2\"}]");
        let report = sink.report();
        assert_eq!(report.losses.len(), 1);
        assert_eq!(report.losses[0].location.line, 2);
        assert!(report.losses[0].description.contains("2 more"));
    }

    #[test]
    fn values_are_escaped() {
        let (json, _) = convert(b"q\n\"he said \"\"hi\"\"\"\n");
        assert_eq!(json, "[{\"q\":\"he said \\\"hi\\\"\"}]");
    }

    #[test]
    fn declares_contract() {
        let converter = CsvToJson;
        assert_eq!(converter.name(), "csv-to-json");
        assert_eq!(converter.from().id, "csv");
        assert_eq!(converter.to().id, "json");
        assert_eq!(converter.fidelity(), Fidelity::Lossless);
        assert_eq!(converter.tier(), Tier::Native);
    }
}
