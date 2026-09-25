//! The row formats among themselves: CSV <-> TSV (the same records with
//! the other separator) and JSON Lines <-> JSON (one value per line against
//! one array), all streamed.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::{CsvReader, CsvWriter, Record};
use crate::io::json::copy::copy_value;
use crate::io::json::{JsonTokenizer, JsonWriter, Token};

const PROGRESS_INTERVAL: u64 = 4096;

/// CSV and TSV into each other: records read with one separator, written
/// with the other, quoted where the target needs it.
pub struct Delimited {
    pub name: &'static str,
    pub from: &'static Format,
    pub to: &'static Format,
    pub read_delimiter: u8,
    pub write_delimiter: u8,
}

pub static CSV_TO_TSV: Delimited = Delimited {
    name: "csv-to-tsv",
    from: &formats::CSV,
    to: &formats::TSV,
    read_delimiter: b',',
    write_delimiter: b'\t',
};

pub static TSV_TO_CSV: Delimited = Delimited {
    name: "tsv-to-csv",
    from: &formats::TSV,
    to: &formats::CSV,
    read_delimiter: b'\t',
    write_delimiter: b',',
};

impl Converter for Delimited {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        self.to
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
        let mut reader = CsvReader::with_delimiter(input, self.read_delimiter);
        let mut writer = CsvWriter::with_delimiter(output, self.write_delimiter);
        let mut record = Record::new();
        let mut count: u64 = 0;
        while reader.read_record(&mut record)? {
            writer.write_record(record.fields())?;
            count += 1;
            if count % PROGRESS_INTERVAL == 0 {
                context.progress(self.name, reader.bytes_consumed());
            }
        }
        writer.flush()?;
        Ok(())
    }
}

/// JSON Lines -> JSON: the values, in order, as one array.
pub struct JsonlToJson;

impl Converter for JsonlToJson {
    fn name(&self) -> &'static str {
        "jsonl-to-json"
    }

    fn from(&self) -> &'static Format {
        &formats::JSONL
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
        let mut tokens = JsonTokenizer::new(input);
        let mut writer = JsonWriter::new(output);
        writer.begin_array()?;
        let mut count: u64 = 0;
        loop {
            let first = tokens.next_token()?;
            if first == Token::End {
                break;
            }
            copy_value(&mut tokens, first, &mut writer)?;
            count += 1;
            if count % PROGRESS_INTERVAL == 0 {
                context.progress(self.name(), tokens.bytes_consumed());
            }
        }
        writer.end_array()?;
        writer.flush()?;
        Ok(())
    }
}

/// JSON -> JSON Lines: an array's elements one per line; any other root
/// on one line.
pub struct JsonToJsonl;

const JSON_TO_JSONL_NOTE: &str =
    "an array becomes one line per element; any other root becomes one line";

impl Converter for JsonToJsonl {
    fn name(&self) -> &'static str {
        "json-to-jsonl"
    }

    fn from(&self) -> &'static Format {
        &formats::JSON
    }

    fn to(&self) -> &'static Format {
        &formats::JSONL
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(JSON_TO_JSONL_NOTE)
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
        let mut tokens = JsonTokenizer::new(input);
        let mut writer = JsonWriter::new(output);
        let root = tokens.next_token()?;
        if root == Token::End {
            writer.flush()?;
            return Ok(());
        }
        if root != Token::BeginArray {
            copy_value(&mut tokens, root, &mut writer)?;
            writer.raw("\n")?;
            expect_end(&mut tokens)?;
            writer.flush()?;
            return Ok(());
        }
        let mut expect_value = true;
        let mut count: u64 = 0;
        loop {
            let token = tokens.next_token()?;
            match token {
                Token::EndArray if !expect_value || count == 0 => break,
                Token::Comma if !expect_value => expect_value = true,
                other if expect_value => {
                    copy_value(&mut tokens, other, &mut writer)?;
                    writer.raw("\n")?;
                    expect_value = false;
                    count += 1;
                    if count % PROGRESS_INTERVAL == 0 {
                        context.progress(self.name(), tokens.bytes_consumed());
                    }
                }
                other => {
                    return Err(ConvertError::Malformed {
                        location: tokens.location(),
                        message: format!("unexpected {}", other.describe()),
                    });
                }
            }
        }
        expect_end(&mut tokens)?;
        writer.flush()?;
        Ok(())
    }
}

fn expect_end<R: std::io::Read>(tokens: &mut JsonTokenizer<R>) -> Result<(), ConvertError> {
    match tokens.next_token()? {
        Token::End => Ok(()),
        other => Err(ConvertError::Malformed {
            location: tokens.location(),
            message: format!("unexpected {} after the document", other.describe()),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::ConvertOptions;
    use crate::event::NullSink;

    fn run(converter: &dyn Converter, input: &[u8]) -> Result<String, ConvertError> {
        let options = ConvertOptions::default();
        let mut sink = NullSink;
        let mut context = Context::new(&mut sink, &options);
        let mut output = Vec::new();
        let mut cursor = std::io::Cursor::new(input);
        converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
        Ok(String::from_utf8(output).unwrap_or_default())
    }

    #[test]
    fn csv_and_tsv_swap_separators_and_quote_for_the_target() {
        assert_eq!(
            run(&CSV_TO_TSV, b"a,b\n\"x,y\",\"t\tu\"\n").unwrap(),
            "a\tb\nx,y\t\"t\tu\"\n"
        );
        assert_eq!(
            run(&TSV_TO_CSV, b"a\tb\nx,y\t\"t\tu\"\n").unwrap(),
            "a,b\n\"x,y\",t\tu\n"
        );
    }

    #[test]
    fn jsonl_and_json_swap_lines_and_arrays() {
        assert_eq!(
            run(&JsonlToJson, b"{\"a\":1}\n[2, 3]\n\n\"s\"\n").unwrap(),
            r#"[{"a":1},[2,3],"s"]"#
        );
        assert_eq!(
            run(&JsonToJsonl, b"[{\"a\":1}, [2,3], \"s\"]").unwrap(),
            "{\"a\":1}\n[2,3]\n\"s\"\n"
        );
        assert_eq!(run(&JsonToJsonl, b"{\"a\":1}").unwrap(), "{\"a\":1}\n");
        assert_eq!(run(&JsonlToJson, b"").unwrap(), "[]");
    }

    #[test]
    fn a_bad_line_is_malformed_at_its_line() {
        let error = run(&JsonlToJson, b"{\"a\":1}\n{\"b\":\n").unwrap_err();
        assert!(
            matches!(error, ConvertError::Malformed { location, .. } if location.line == 3),
            "{error}"
        );
    }
}
