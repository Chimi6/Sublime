//! CSV writer. Quotes only when needed. LF line endings.

use std::io::{self, Write};

pub struct CsvWriter<W: Write> {
    sink: W,
    delimiter: u8,
}

impl<W: Write> CsvWriter<W> {
    pub fn new(sink: W) -> Self {
        CsvWriter::with_delimiter(sink, b',')
    }

    pub fn with_delimiter(sink: W, delimiter: u8) -> Self {
        CsvWriter { sink, delimiter }
    }

    pub fn write_record<'a, I>(&mut self, fields: I) -> io::Result<()>
    where
        I: IntoIterator<Item = &'a str>,
    {
        let mut field_count = 0usize;
        let mut only_field_was_empty = false;
        for field in fields {
            if field_count > 0 {
                self.sink.write_all(&[self.delimiter])?;
            }
            self.write_field(field)?;
            only_field_was_empty = field.is_empty();
            field_count += 1;
        }
        let is_single_empty_field = field_count == 1 && only_field_was_empty;
        if is_single_empty_field {
            self.sink.write_all(b"\"\"")?;
        }
        self.sink.write_all(b"\n")
    }

    pub fn flush(&mut self) -> io::Result<()> {
        self.sink.flush()
    }

    pub fn into_inner(self) -> W {
        self.sink
    }

    fn write_field(&mut self, field: &str) -> io::Result<()> {
        if !needs_quoting(field, self.delimiter) {
            return self.sink.write_all(field.as_bytes());
        }
        self.sink.write_all(b"\"")?;
        let mut is_first_piece = true;
        for piece in field.split('"') {
            if !is_first_piece {
                self.sink.write_all(b"\"\"")?;
            }
            is_first_piece = false;
            self.sink.write_all(piece.as_bytes())?;
        }
        self.sink.write_all(b"\"")
    }
}

fn needs_quoting(field: &str, delimiter: u8) -> bool {
    for byte in field.bytes() {
        let is_special = byte == delimiter || matches!(byte, b'"' | b'\n' | b'\r');
        if is_special {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_rows(rows: &[&[&str]]) -> String {
        let mut writer = CsvWriter::new(Vec::new());
        for row in rows {
            writer.write_record(row.iter().copied()).unwrap();
        }
        let bytes = writer.into_inner();
        String::from_utf8(bytes).unwrap()
    }

    #[test]
    fn plain_fields_are_not_quoted() {
        assert_eq!(write_rows(&[&["a", "b", "c"]]), "a,b,c\n");
    }

    #[test]
    fn fields_with_special_characters_are_quoted() {
        let output = write_rows(&[&["x,y", "say \"hi\"", "line1\nline2", "cr\rhere"]]);
        assert_eq!(
            output,
            "\"x,y\",\"say \"\"hi\"\"\",\"line1\nline2\",\"cr\rhere\"\n"
        );
    }

    #[test]
    fn empty_fields_between_others_are_bare() {
        assert_eq!(write_rows(&[&["a", "", "c"]]), "a,,c\n");
    }

    #[test]
    fn single_empty_field_is_written_as_quotes() {
        assert_eq!(write_rows(&[&[""]]), "\"\"\n");
    }

    #[test]
    fn round_trips_through_the_reader() {
        let rows: Vec<Vec<String>> = vec![
            vec!["name".to_string(), "note".to_string()],
            vec!["a,b".to_string(), "he said \"no\"\nthen left".to_string()],
            vec!["".to_string(), "".to_string()],
        ];
        let mut writer = CsvWriter::new(Vec::new());
        for row in &rows {
            writer.write_record(row.iter().map(String::as_str)).unwrap();
        }
        let bytes = writer.into_inner();
        let mut reader = super::super::CsvReader::new(&bytes[..]);
        let mut record = super::super::Record::new();
        let mut read_back = Vec::new();
        while reader.read_record(&mut record).unwrap() {
            let fields: Vec<String> = record.fields().map(str::to_string).collect();
            read_back.push(fields);
        }
        assert_eq!(read_back, rows);
    }
}
