//! Streaming CSV reader. Reads byte by byte from any `Read`, reusing one
//! `Record` buffer so steady-state parsing allocates nothing.

use std::fmt;
use std::io::{self, Read};

use crate::io::scan::find_csv_delimiter;

const BUFFER_SIZE: usize = 64 * 1024;
const UTF8_BOM: [u8; 3] = [0xEF, 0xBB, 0xBF];

#[derive(Debug)]
pub enum CsvError {
    Io(io::Error),
    InvalidUtf8 { line: u64 },
    UnterminatedQuote { line: u64 },
}

impl From<io::Error> for CsvError {
    fn from(error: io::Error) -> Self {
        CsvError::Io(error)
    }
}

impl fmt::Display for CsvError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CsvError::Io(error) => write!(formatter, "{error}"),
            CsvError::InvalidUtf8 { line } => {
                write!(formatter, "line {line}: invalid UTF-8")
            }
            CsvError::UnterminatedQuote { line } => {
                write!(formatter, "line {line}: quoted field never closed")
            }
        }
    }
}

impl std::error::Error for CsvError {}

/// One parsed record. Reuse it across `read_record` calls.
#[derive(Debug, Default)]
pub struct Record {
    data: String,
    bounds: Vec<(usize, usize)>,
    line: u64,
}

impl Record {
    pub fn new() -> Self {
        Record::default()
    }

    pub fn len(&self) -> usize {
        self.bounds.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bounds.is_empty()
    }

    pub fn field(&self, index: usize) -> Option<&str> {
        let (start, end) = *self.bounds.get(index)?;
        Some(&self.data[start..end])
    }

    pub fn fields(&self) -> impl Iterator<Item = &str> {
        self.bounds
            .iter()
            .map(move |(start, end)| &self.data[*start..*end])
    }

    /// 1-based line on which this record started.
    pub fn line(&self) -> u64 {
        self.line
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    FieldStart,
    Unquoted,
    Quoted,
    AfterQuote,
}

pub struct CsvReader<R: Read> {
    source: R,
    buffer: Vec<u8>,
    position: usize,
    filled: usize,
    reached_end: bool,
    checked_bom: bool,
    line: u64,
    bytes_consumed: u64,
}

impl<R: Read> CsvReader<R> {
    pub fn new(source: R) -> Self {
        CsvReader {
            source,
            buffer: vec![0; BUFFER_SIZE],
            position: 0,
            filled: 0,
            reached_end: false,
            checked_bom: false,
            line: 1,
            bytes_consumed: 0,
        }
    }

    pub fn bytes_consumed(&self) -> u64 {
        self.bytes_consumed
    }

    /// Reads the next record into `record`. Returns `Ok(false)` at end of input.
    pub fn read_record(&mut self, record: &mut Record) -> Result<bool, CsvError> {
        if !self.checked_bom {
            self.skip_bom()?;
        }
        let mut data = std::mem::take(&mut record.data).into_bytes();
        data.clear();
        record.bounds.clear();
        record.line = self.line;

        let mut state = State::FieldStart;
        let mut field_start = 0usize;
        let mut finished = false;

        while !finished {
            let byte = match self.next_byte()? {
                Some(byte) => byte,
                None => break,
            };
            match state {
                State::FieldStart => {
                    if byte == b'"' {
                        state = State::Quoted;
                    } else if byte == b',' {
                        record.bounds.push((field_start, data.len()));
                        field_start = data.len();
                    } else if byte == b'\n' || byte == b'\r' {
                        self.consume_line_ending(byte)?;
                        let is_blank_line = record.bounds.is_empty() && data.is_empty();
                        if is_blank_line {
                            record.line = self.line;
                        } else {
                            record.bounds.push((field_start, data.len()));
                            finished = true;
                        }
                    } else {
                        data.push(byte);
                        state = State::Unquoted;
                    }
                }
                State::Unquoted => {
                    if byte == b',' {
                        record.bounds.push((field_start, data.len()));
                        field_start = data.len();
                        state = State::FieldStart;
                    } else if byte == b'\n' || byte == b'\r' {
                        self.consume_line_ending(byte)?;
                        record.bounds.push((field_start, data.len()));
                        finished = true;
                    } else {
                        data.push(byte);
                        self.copy_run(&mut data)?;
                    }
                }
                State::Quoted => {
                    if byte == b'"' {
                        state = State::AfterQuote;
                    } else {
                        if byte == b'\n' {
                            self.line += 1;
                        }
                        data.push(byte);
                        self.copy_run(&mut data)?;
                    }
                }
                State::AfterQuote => {
                    if byte == b'"' {
                        data.push(b'"');
                        state = State::Quoted;
                    } else if byte == b',' {
                        record.bounds.push((field_start, data.len()));
                        field_start = data.len();
                        state = State::FieldStart;
                    } else if byte == b'\n' || byte == b'\r' {
                        self.consume_line_ending(byte)?;
                        record.bounds.push((field_start, data.len()));
                        finished = true;
                    } else {
                        data.push(byte);
                        state = State::Unquoted;
                    }
                }
            }
        }

        if !finished {
            if state == State::Quoted {
                return Err(CsvError::UnterminatedQuote { line: record.line });
            }
            let has_content = state != State::FieldStart || !record.bounds.is_empty();
            if !has_content {
                record.data = restore_data(data, record.line)?;
                return Ok(false);
            }
            record.bounds.push((field_start, data.len()));
        }

        record.data = restore_data(data, record.line)?;
        Ok(true)
    }

    fn consume_line_ending(&mut self, byte: u8) -> io::Result<()> {
        self.line += 1;
        if byte == b'\r' {
            let next = self.peek_byte()?;
            if next == Some(b'\n') {
                self.next_byte()?;
            }
        }
        Ok(())
    }

    fn skip_bom(&mut self) -> io::Result<()> {
        self.checked_bom = true;
        while self.filled < UTF8_BOM.len() && !self.reached_end {
            let read_count = self.source.read(&mut self.buffer[self.filled..])?;
            if read_count == 0 {
                self.reached_end = true;
            } else {
                self.filled += read_count;
            }
        }
        let head = &self.buffer[..self.filled];
        if head.starts_with(&UTF8_BOM) {
            self.position = UTF8_BOM.len();
            self.bytes_consumed = UTF8_BOM.len() as u64;
        }
        Ok(())
    }

    /// Copies bytes into `data` up to, but not including, the next `,`, `"`,
    /// `\n`, or `\r`, refilling the buffer as needed. The delimiter itself is
    /// left for `next_byte` so the state machine handles it. Ordinary bytes
    /// are the common case, so this is where most of the input flows.
    fn copy_run(&mut self, data: &mut Vec<u8>) -> io::Result<()> {
        loop {
            if self.position == self.filled {
                let has_more = self.fill()?;
                if !has_more {
                    return Ok(());
                }
            }
            let available = &self.buffer[self.position..self.filled];
            let run_length = match find_csv_delimiter(available) {
                Some(index) => index,
                None => available.len(),
            };
            data.extend_from_slice(&available[..run_length]);
            self.position += run_length;
            self.bytes_consumed += run_length as u64;
            let stopped_at_delimiter = run_length < available.len();
            if stopped_at_delimiter {
                return Ok(());
            }
        }
    }

    /// Refills the buffer. Returns `Ok(false)` when the source is exhausted.
    fn fill(&mut self) -> io::Result<bool> {
        if self.reached_end {
            return Ok(false);
        }
        let read_count = self.source.read(&mut self.buffer)?;
        if read_count == 0 {
            self.reached_end = true;
            return Ok(false);
        }
        self.position = 0;
        self.filled = read_count;
        Ok(true)
    }

    fn next_byte(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.filled {
            let has_more = self.fill()?;
            if !has_more {
                return Ok(None);
            }
        }
        let byte = self.buffer[self.position];
        self.position += 1;
        self.bytes_consumed += 1;
        Ok(Some(byte))
    }

    fn peek_byte(&mut self) -> io::Result<Option<u8>> {
        if self.position == self.filled {
            let has_more = self.fill()?;
            if !has_more {
                return Ok(None);
            }
        }
        Ok(Some(self.buffer[self.position]))
    }
}

fn restore_data(data: Vec<u8>, line: u64) -> Result<String, CsvError> {
    match String::from_utf8(data) {
        Ok(text) => Ok(text),
        Err(_) => Err(CsvError::InvalidUtf8 { line }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read_all(input: &[u8]) -> Result<Vec<Vec<String>>, CsvError> {
        let mut reader = CsvReader::new(input);
        let mut record = Record::new();
        let mut rows = Vec::new();
        loop {
            let has_record = reader.read_record(&mut record)?;
            if !has_record {
                break;
            }
            let fields: Vec<String> = record.fields().map(str::to_string).collect();
            rows.push(fields);
        }
        Ok(rows)
    }

    fn row(fields: &[&str]) -> Vec<String> {
        fields.iter().map(|field| field.to_string()).collect()
    }

    #[test]
    fn reads_simple_rows() {
        let rows = read_all(b"a,b,c\n1,2,3\n").unwrap();
        assert_eq!(rows, vec![row(&["a", "b", "c"]), row(&["1", "2", "3"])]);
    }

    #[test]
    fn last_row_without_trailing_newline() {
        let rows = read_all(b"a,b\n1,2").unwrap();
        assert_eq!(rows, vec![row(&["a", "b"]), row(&["1", "2"])]);
    }

    #[test]
    fn quoted_fields_keep_commas_and_newlines() {
        let rows = read_all(b"\"x,y\",\"line1\nline2\"\n").unwrap();
        assert_eq!(rows, vec![row(&["x,y", "line1\nline2"])]);
    }

    #[test]
    fn doubled_quote_is_an_escaped_quote() {
        let rows = read_all(b"\"say \"\"hi\"\"\",b\n").unwrap();
        assert_eq!(rows, vec![row(&["say \"hi\"", "b"])]);
    }

    #[test]
    fn crlf_line_endings() {
        let rows = read_all(b"a,b\r\n1,2\r\n").unwrap();
        assert_eq!(rows, vec![row(&["a", "b"]), row(&["1", "2"])]);
    }

    #[test]
    fn strips_utf8_bom() {
        let rows = read_all(b"\xEF\xBB\xBFa,b\n").unwrap();
        assert_eq!(rows, vec![row(&["a", "b"])]);
    }

    #[test]
    fn skips_blank_lines() {
        let rows = read_all(b"a,b\n\n\n1,2\n\n").unwrap();
        assert_eq!(rows, vec![row(&["a", "b"]), row(&["1", "2"])]);
    }

    #[test]
    fn empty_fields_are_preserved() {
        let rows = read_all(b"a,,c\n,b,\n").unwrap();
        assert_eq!(rows, vec![row(&["a", "", "c"]), row(&["", "b", ""])]);
    }

    #[test]
    fn quoted_empty_field_is_a_record_with_one_empty_field() {
        let rows = read_all(b"\"\"\n").unwrap();
        assert_eq!(rows, vec![row(&[""])]);
    }

    #[test]
    fn stray_text_after_closing_quote_is_kept() {
        let rows = read_all(b"\"a\"b,c\n").unwrap();
        assert_eq!(rows, vec![row(&["ab", "c"])]);
    }

    #[test]
    fn unterminated_quote_is_an_error() {
        let error = read_all(b"a,\"open\n").unwrap_err();
        let is_unterminated = matches!(error, CsvError::UnterminatedQuote { line: 1 });
        assert!(is_unterminated);
    }

    #[test]
    fn invalid_utf8_is_an_error_with_line() {
        let error = read_all(b"ok\n\xFF\xFE\n").unwrap_err();
        let is_invalid = matches!(error, CsvError::InvalidUtf8 { line: 2 });
        assert!(is_invalid);
    }

    #[test]
    fn empty_input_has_no_records() {
        let rows = read_all(b"").unwrap();
        assert!(rows.is_empty());
    }

    #[test]
    fn record_reports_its_line_and_reader_counts_bytes() {
        let mut reader = CsvReader::new(&b"a,b\n\n1,2\n"[..]);
        let mut record = Record::new();
        reader.read_record(&mut record).unwrap();
        assert_eq!(record.line(), 1);
        reader.read_record(&mut record).unwrap();
        assert_eq!(record.line(), 3);
        assert_eq!(record.field(1), Some("2"));
        assert_eq!(record.field(2), None);
        assert_eq!(reader.bytes_consumed(), 9);
    }

    #[test]
    fn works_across_small_read_chunks() {
        struct OneByte<'a> {
            data: &'a [u8],
        }
        impl std::io::Read for OneByte<'_> {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.data.is_empty() || buffer.is_empty() {
                    return Ok(0);
                }
                buffer[0] = self.data[0];
                self.data = &self.data[1..];
                Ok(1)
            }
        }
        let source = OneByte {
            data: b"\xEF\xBB\xBF\"a,b\",c\r\n1,2\n",
        };
        let mut reader = CsvReader::new(source);
        let mut record = Record::new();
        let mut rows = Vec::new();
        while reader.read_record(&mut record).unwrap() {
            let fields: Vec<String> = record.fields().map(str::to_string).collect();
            rows.push(fields);
        }
        assert_eq!(rows, vec![row(&["a,b", "c"]), row(&["1", "2"])]);
    }
}
