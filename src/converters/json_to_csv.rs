//! JSON -> CSV. Two passes: collect keys, then write rows. Runs in constant
//! memory from a file; buffers once from a stream.

use std::collections::{HashMap, HashSet};
use std::io::{Read, Write};

use crate::converter::{ConvertError, Converter, Fidelity, Input, RewindableRead, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::csv::CsvWriter;
use crate::io::json::{JsonTokenizer, JsonWriter, Token};

const PROGRESS_INTERVAL: u64 = 4096;
const FIDELITY_NOTE: &str = "nested values are written as JSON text, non-string scalars are written as their JSON text, null becomes an empty cell, and objects with differing key sets get empty cells for missing keys";

pub struct JsonToCsv {
    pub name: &'static str,
    pub from: &'static Format,
    pub to: &'static Format,
    /// `,` or `\t`.
    pub delimiter: u8,
    /// The input is JSON Lines (one object per line) rather than an array.
    pub lines: bool,
}

pub static JSON_TO_CSV: JsonToCsv = JsonToCsv {
    name: "json-to-csv",
    from: &formats::JSON,
    to: &formats::CSV,
    delimiter: b',',
    lines: false,
};

pub static JSON_TO_TSV: JsonToCsv = JsonToCsv {
    name: "json-to-tsv",
    from: &formats::JSON,
    to: &formats::TSV,
    delimiter: b'\t',
    lines: false,
};

pub static JSONL_TO_CSV: JsonToCsv = JsonToCsv {
    name: "jsonl-to-csv",
    from: &formats::JSONL,
    to: &formats::CSV,
    delimiter: b',',
    lines: true,
};

pub static JSONL_TO_TSV: JsonToCsv = JsonToCsv {
    name: "jsonl-to-tsv",
    from: &formats::JSONL,
    to: &formats::TSV,
    delimiter: b'\t',
    lines: true,
};

/// How the rows arrive: as the elements of one array, or one per line.
#[derive(Clone, Copy)]
struct Shape {
    name: &'static str,
    lines: bool,
}

/// Which columns have already reported a scalar or a null loss, so each
/// column reports each once.
struct ColumnReports {
    name: &'static str,
    scalar: Vec<bool>,
    null: Vec<bool>,
}

impl Converter for JsonToCsv {
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
        Fidelity::Conditional(FIDELITY_NOTE)
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
        let shape = Shape {
            name: self.name,
            lines: self.lines,
        };
        let mut rewound = input.into_rewindable()?;
        let keys = collect_keys(&mut rewound, shape, context)?;
        rewound.rewind()?;
        write_rows(&mut rewound, shape, self.delimiter, &keys, output, context)
    }
}

fn unsupported_root(shape: Shape, found: Token) -> ConvertError {
    let message = if shape.lines {
        format!(
            "every JSON Lines value must be an object, found {}",
            found.describe()
        )
    } else {
        format!(
            "JSON root must be an array of objects, found {}",
            found.describe()
        )
    };
    ConvertError::Unsupported(message)
}

/// The next row's opening token, or `None` at the end of the rows: past
/// the closing `]` of the array, or at the end of the lines.
fn next_row<R: Read>(
    tokenizer: &mut JsonTokenizer<R>,
    shape: Shape,
) -> Result<Option<Token>, ConvertError> {
    loop {
        let token = tokenizer.next_token()?;
        match token {
            Token::EndArray if !shape.lines => return Ok(None),
            Token::Comma if !shape.lines => continue,
            Token::End if shape.lines => return Ok(None),
            Token::BeginObject => return Ok(Some(token)),
            other => return Err(unsupported_root(shape, other)),
        }
    }
}

/// Pass one: the ordered union of keys across all objects.
fn collect_keys(
    source: &mut dyn Read,
    shape: Shape,
    context: &mut Context<'_>,
) -> Result<Vec<String>, ConvertError> {
    let mut tokenizer = JsonTokenizer::new(source);
    let mut keys: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut object_index: u64 = 0;
    let mut reported_differing_sets = false;

    if !shape.lines {
        let root = tokenizer.next_token()?;
        if root != Token::BeginArray {
            return Err(unsupported_root(shape, root));
        }
    }
    while next_row(&mut tokenizer, shape)?.is_some() {
        {
            {
                let key_count_before = keys.len();
                let object_key_count = collect_object_keys(&mut tokenizer, &mut keys, &mut seen)?;
                let key_count_after = keys.len();
                let added_new_keys = key_count_after > key_count_before;
                let is_missing_keys = object_key_count < key_count_after;
                let differs = is_missing_keys || (added_new_keys && object_index > 0);
                if differs && !reported_differing_sets {
                    let location = tokenizer.location();
                    context.loss(
                        shape.name,
                        location,
                        "objects have differing key sets; missing keys are written as empty cells",
                    );
                    reported_differing_sets = true;
                }
                object_index += 1;
            }
        }
    }
    Ok(keys)
}

/// Reads one object's keys (after its `{`), skipping values. Returns how
/// many keys the object had.
fn collect_object_keys<R: Read>(
    tokenizer: &mut JsonTokenizer<R>,
    keys: &mut Vec<String>,
    seen: &mut HashSet<String>,
) -> Result<usize, ConvertError> {
    let mut count = 0usize;
    loop {
        let token = tokenizer.next_token()?;
        match token {
            Token::EndObject => break,
            Token::Comma => continue,
            Token::String => {
                if !seen.contains(tokenizer.text()) {
                    let key = tokenizer.text().to_string();
                    seen.insert(key.clone());
                    keys.push(key);
                }
                tokenizer.expect(Token::Colon)?;
                let value_token = tokenizer.next_token()?;
                tokenizer.skip_value(value_token)?;
                count += 1;
            }
            other => {
                let message = format!("expected a key, found {}", other.describe());
                return Err(ConvertError::Malformed {
                    location: tokenizer.location(),
                    message,
                });
            }
        }
    }
    Ok(count)
}

/// Pass two: header and one row per object.
fn write_rows(
    source: &mut dyn Read,
    shape: Shape,
    delimiter: u8,
    keys: &[String],
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let mut tokenizer = JsonTokenizer::new(source);
    let mut writer = CsvWriter::with_delimiter(output, delimiter);
    let mut column_of: HashMap<&str, usize> = HashMap::new();
    for (index, key) in keys.iter().enumerate() {
        column_of.insert(key.as_str(), index);
    }
    let mut row: Vec<String> = vec![String::new(); keys.len()];
    let mut reports = ColumnReports {
        name: shape.name,
        scalar: vec![false; keys.len()],
        null: vec![false; keys.len()],
    };

    if !shape.lines {
        tokenizer.expect(Token::BeginArray)?;
    }
    let mut row_count: u64 = 0;
    while next_row(&mut tokenizer, shape)?.is_some() {
        {
            {
                if row_count == 0 {
                    writer.write_record(keys.iter().map(String::as_str))?;
                }
                for cell in row.iter_mut() {
                    cell.clear();
                }
                fill_row(&mut tokenizer, &column_of, &mut row, &mut reports, context)?;
                writer.write_record(row.iter().map(String::as_str))?;
                row_count += 1;
                if row_count % PROGRESS_INTERVAL == 0 {
                    context.progress(shape.name, tokenizer.bytes_consumed());
                }
            }
        }
    }
    writer.flush()?;
    Ok(())
}

fn fill_row<R: Read>(
    tokenizer: &mut JsonTokenizer<R>,
    column_of: &HashMap<&str, usize>,
    row: &mut [String],
    reports: &mut ColumnReports,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    loop {
        let token = tokenizer.next_token()?;
        match token {
            Token::EndObject => break,
            Token::Comma => continue,
            Token::String => {
                let column = column_of.get(tokenizer.text()).copied();
                tokenizer.expect(Token::Colon)?;
                let value_token = tokenizer.next_token()?;
                let column = match column {
                    Some(column) => column,
                    None => {
                        tokenizer.skip_value(value_token)?;
                        continue;
                    }
                };
                row[column].clear();
                write_cell(tokenizer, value_token, row, column, reports, context)?;
            }
            other => {
                let message = format!("expected a key, found {}", other.describe());
                return Err(ConvertError::Malformed {
                    location: tokenizer.location(),
                    message,
                });
            }
        }
    }
    Ok(())
}

fn write_cell<R: Read>(
    tokenizer: &mut JsonTokenizer<R>,
    token: Token,
    row: &mut [String],
    column: usize,
    reports: &mut ColumnReports,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    match token {
        Token::String => {
            row[column].push_str(tokenizer.text());
        }
        Token::Number | Token::True | Token::False => {
            row[column].push_str(tokenizer.text());
            report_column_once(
                reports.name,
                tokenizer,
                column,
                &mut reports.scalar,
                context,
                "non-string scalar written as its JSON text",
            );
        }
        Token::Null => {
            report_column_once(
                reports.name,
                tokenizer,
                column,
                &mut reports.null,
                context,
                "null written as an empty cell",
            );
        }
        Token::BeginObject | Token::BeginArray => {
            render_value(tokenizer, token, &mut row[column])?;
            let location = tokenizer.location();
            context.loss(reports.name, location, "nested value written as JSON text");
        }
        other => {
            let message = format!("expected a value, found {}", other.describe());
            return Err(ConvertError::Malformed {
                location: tokenizer.location(),
                message,
            });
        }
    }
    Ok(())
}

fn report_column_once<R: Read>(
    name: &'static str,
    tokenizer: &JsonTokenizer<R>,
    column: usize,
    column_loss_reported: &mut [bool],
    context: &mut Context<'_>,
    description: &str,
) {
    if column_loss_reported[column] {
        return;
    }
    column_loss_reported[column] = true;
    let location = tokenizer.location();
    let message = format!("column {}: {description}", column + 1);
    context.loss(name, location, message);
}

/// One open container while re-serializing a nested value.
struct Frame {
    is_object: bool,
    expecting_key: bool,
}

/// Re-serializes the nested value that `first` started into `out`.
fn render_value<R: Read>(
    tokenizer: &mut JsonTokenizer<R>,
    first: Token,
    out: &mut String,
) -> Result<(), ConvertError> {
    let mut buffer: Vec<u8> = Vec::new();
    let mut writer = JsonWriter::new(&mut buffer);
    let mut frames: Vec<Frame> = Vec::new();
    let mut token = first;
    loop {
        match token {
            Token::BeginObject => {
                writer.begin_object()?;
                frames.push(Frame {
                    is_object: true,
                    expecting_key: true,
                });
            }
            Token::BeginArray => {
                writer.begin_array()?;
                frames.push(Frame {
                    is_object: false,
                    expecting_key: false,
                });
            }
            Token::EndObject => {
                writer.end_object()?;
                frames.pop();
                mark_value_done(&mut frames);
            }
            Token::EndArray => {
                writer.end_array()?;
                frames.pop();
                mark_value_done(&mut frames);
            }
            Token::String => {
                let is_key = match frames.last() {
                    Some(frame) => frame.is_object && frame.expecting_key,
                    None => false,
                };
                if is_key {
                    writer.key(tokenizer.text())?;
                    if let Some(frame) = frames.last_mut() {
                        frame.expecting_key = false;
                    }
                } else {
                    writer.string(tokenizer.text())?;
                    mark_value_done(&mut frames);
                }
            }
            Token::Number | Token::True | Token::False | Token::Null => {
                writer.raw(tokenizer.text())?;
                mark_value_done(&mut frames);
            }
            Token::Colon | Token::Comma => {}
            Token::End => {
                return Err(ConvertError::Malformed {
                    location: tokenizer.location(),
                    message: "unexpected end of input inside a nested value".to_string(),
                });
            }
        }
        if frames.is_empty() {
            break;
        }
        token = tokenizer.next_token()?;
    }
    writer.flush()?;
    drop(writer);
    match std::str::from_utf8(&buffer) {
        Ok(text) => {
            out.push_str(text);
            Ok(())
        }
        Err(_) => Err(ConvertError::Unsupported(
            "internal error: rendered JSON was not UTF-8".to_string(),
        )),
    }
}

/// After a value inside an object, the next string is a key again. Arrays
/// never expect keys.
fn mark_value_done(frames: &mut [Frame]) {
    if let Some(frame) = frames.last_mut() {
        if frame.is_object {
            frame.expecting_key = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{ConvertOptions, FidelityKind};
    use crate::event::CollectingSink;
    use std::io::Cursor;

    fn convert(input: &[u8]) -> Result<(String, CollectingSink), ConvertError> {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        {
            let mut context = Context::new(&mut sink, &options);
            let mut source: &[u8] = input;
            let converter = &JSON_TO_CSV;
            converter.convert(Input::Stream(&mut source), &mut output, &mut context)?;
        }
        Ok((String::from_utf8(output).unwrap(), sink))
    }

    #[test]
    fn flat_objects_become_rows() {
        let (csv, sink) =
            convert(b"[{\"a\":\"1\",\"b\":\"2\"},{\"a\":\"3\",\"b\":\"4\"}]").unwrap();
        assert_eq!(csv, "a,b\n1,2\n3,4\n");
        assert!(sink.report().is_lossless());
    }

    #[test]
    fn key_union_keeps_first_seen_order_and_reports_differing_sets() {
        let (csv, sink) = convert(b"[{\"a\":\"1\"},{\"b\":\"2\",\"a\":\"3\"}]").unwrap();
        assert_eq!(csv, "a,b\n1,\n3,2\n");
        let report = sink.report();
        assert_eq!(report.losses.len(), 1);
        assert!(report.losses[0].description.contains("differing key sets"));
    }

    #[test]
    fn scalars_are_written_as_json_text_with_one_loss_per_column() {
        let (csv, sink) = convert(b"[{\"n\":1.5,\"t\":true},{\"n\":2,\"t\":false}]").unwrap();
        assert_eq!(csv, "n,t\n1.5,true\n2,false\n");
        assert_eq!(sink.report().losses.len(), 2);
    }

    #[test]
    fn null_is_an_empty_cell() {
        let (csv, sink) = convert(b"[{\"a\":null,\"b\":\"x\"}]").unwrap();
        assert_eq!(csv, "a,b\n,x\n");
        assert_eq!(sink.report().losses.len(), 1);
    }

    #[test]
    fn mixed_scalar_and_null_column_reports_both_losses() {
        let (csv, sink) = convert(b"[{\"a\":1},{\"a\":null}]").unwrap();
        // A single-field row with an empty value is written as `""` by
        // `CsvWriter` (see `single_empty_field_is_written_as_quotes`), so
        // it round-trips as an empty cell rather than an empty record.
        assert_eq!(csv, "a\n1\n\"\"\n");
        assert_eq!(sink.report().losses.len(), 2);
    }

    #[test]
    fn nested_values_are_serialized_with_one_loss_each() {
        let (csv, sink) = convert(b"[{\"a\":{\"x\":[1,\"y\"]}},{\"a\":[]}]").unwrap();
        assert_eq!(csv, "a\n\"{\"\"x\"\":[1,\"\"y\"\"]}\"\n[]\n");
        assert_eq!(sink.report().losses.len(), 2);
    }

    #[test]
    fn empty_array_writes_nothing() {
        let (csv, _) = convert(b"[]").unwrap();
        assert_eq!(csv, "");
    }

    #[test]
    fn non_array_root_is_unsupported() {
        let error = convert(b"{\"a\":1}").unwrap_err();
        let is_unsupported = matches!(error, ConvertError::Unsupported(_));
        assert!(is_unsupported);
    }

    #[test]
    fn non_object_element_is_unsupported() {
        let error = convert(b"[1,2]").unwrap_err();
        let is_unsupported = matches!(error, ConvertError::Unsupported(_));
        assert!(is_unsupported);
    }

    #[test]
    fn rewindable_input_is_read_twice_without_buffering() {
        let mut cursor = Cursor::new(b"[{\"a\":\"1\"}]".to_vec());
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        let mut context = Context::new(&mut sink, &options);
        JSON_TO_CSV
            .convert(Input::Rewindable(&mut cursor), &mut output, &mut context)
            .unwrap();
        assert_eq!(String::from_utf8(output).unwrap(), "a\n1\n");
    }

    #[test]
    fn declares_contract() {
        let converter = &JSON_TO_CSV;
        assert_eq!(converter.name(), "json-to-csv");
        assert_eq!(converter.from().id, "json");
        assert_eq!(converter.to().id, "csv");
        assert_eq!(converter.fidelity().kind(), FidelityKind::Conditional);
        assert_eq!(converter.tier(), Tier::Native);
    }
}
