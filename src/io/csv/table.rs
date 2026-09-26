//! Rows as a Markdown table and back: the bridge between the row formats
//! and the document formats. `emit_table` turns records into the event
//! stream's table (first record as the header, every row padded or
//! trimmed to the header's width, line breaks inside cells folded to
//! spaces); `TableRows` is a sink that writes the first table it sees out
//! as records and ignores everything else.

use std::borrow::Cow;
use std::io::{self, Read, Write};

use crate::io::csv::{CsvError, CsvReader, CsvWriter, Record};
use crate::io::markdown::{Alignment, Event, EventSink, Tag, TagEnd};

/// What `emit_table` had to change, reported per document.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct TableNotes {
    /// Rows with more cells than the header, whose extra cells were dropped.
    pub trimmed_rows: u64,
    /// Cells whose line breaks were folded to spaces.
    pub folded_cells: u64,
}

/// Reads every record and pushes one table.
pub fn emit_table<R: Read>(
    reader: &mut CsvReader<R>,
    sink: &mut dyn EventSink<'_>,
) -> Result<TableNotes, CsvError> {
    let mut notes = TableNotes::default();
    let mut record = Record::new();
    if !reader.read_record(&mut record)? {
        return Ok(notes);
    }
    let width = record.len();
    sink.event(Event::Start(Tag::Table(vec![Alignment::None; width])));
    sink.event(Event::Start(Tag::TableHead));
    emit_cells(&record, width, sink, &mut notes);
    sink.event(Event::End(TagEnd::TableHead));
    while reader.read_record(&mut record)? {
        sink.event(Event::Start(Tag::TableRow));
        emit_cells(&record, width, sink, &mut notes);
        sink.event(Event::End(TagEnd::TableRow));
    }
    sink.event(Event::End(TagEnd::Table));
    Ok(notes)
}

fn emit_cells(record: &Record, width: usize, sink: &mut dyn EventSink<'_>, notes: &mut TableNotes) {
    if record.len() > width {
        notes.trimmed_rows += 1;
    }
    for index in 0..width {
        sink.event(Event::Start(Tag::TableCell));
        let cell = record.field(index).unwrap_or("");
        if !cell.is_empty() {
            let has_break = cell.bytes().any(|byte| byte == b'\n' || byte == b'\r');
            if has_break {
                notes.folded_cells += 1;
                let folded = fold_breaks(cell);
                sink.transient(Event::Text(Cow::Owned(folded)));
            } else {
                sink.transient(Event::Text(Cow::Borrowed(cell)));
            }
        }
        sink.event(Event::End(TagEnd::TableCell));
    }
}

/// Line breaks become single spaces, as a table cell cannot hold them.
fn fold_breaks(cell: &str) -> String {
    let mut out = String::with_capacity(cell.len());
    let mut pending_space = false;
    for character in cell.chars() {
        if character == '\n' || character == '\r' {
            pending_space = true;
            continue;
        }
        if pending_space {
            if !out.is_empty() && !out.ends_with(' ') && character != ' ' {
                out.push(' ');
            }
            pending_space = false;
        }
        out.push(character);
    }
    out
}

/// Writes the first table of a document as records; everything else in
/// the document is skipped. Inline formatting is flattened to its text.
pub struct TableRows<W: Write> {
    writer: CsvWriter<W>,
    /// Cell buffers reused row after row; `count` says how many are live.
    cells: Vec<String>,
    count: usize,
    current: String,
    depth: u32,
    in_table: bool,
    done: bool,
    failure: Option<io::Error>,
}

impl<W: Write> TableRows<W> {
    pub fn new(sink: W, delimiter: u8) -> TableRows<W> {
        TableRows {
            writer: CsvWriter::with_delimiter(sink, delimiter),
            cells: Vec::new(),
            count: 0,
            current: String::new(),
            depth: 0,
            in_table: false,
            done: false,
            failure: None,
        }
    }

    /// True when a table was written.
    pub fn found_table(&self) -> bool {
        self.done
    }

    pub fn finish(mut self) -> io::Result<()> {
        if let Some(failure) = self.failure.take() {
            return Err(failure);
        }
        self.writer.flush()
    }

    fn write_row(&mut self) {
        if self.failure.is_some() {
            return;
        }
        let row = self.cells[..self.count].iter().map(String::as_str);
        if let Err(error) = self.writer.write_record(row) {
            self.failure = Some(error);
        }
        self.count = 0;
    }

    fn end_cell(&mut self) {
        if self.count == self.cells.len() {
            self.cells.push(String::new());
        }
        let cell = &mut self.cells[self.count];
        cell.clear();
        std::mem::swap(cell, &mut self.current);
        self.current.clear();
        self.count += 1;
    }
}

impl<W: Write> EventSink<'_> for TableRows<W> {
    /// Cell text is copied into the row as it arrives, so a transient
    /// event needs no owned copy first.
    fn transient(&mut self, event: Event<'_>) {
        self.event(event);
    }

    fn event(&mut self, event: Event<'_>) {
        if self.done {
            return;
        }
        match event {
            Event::Start(Tag::Table(_)) => {
                if self.depth == 0 {
                    self.in_table = true;
                }
                self.depth += 1;
            }
            Event::End(TagEnd::Table) => {
                self.depth = self.depth.saturating_sub(1);
                if self.depth == 0 && self.in_table {
                    self.in_table = false;
                    self.done = true;
                }
            }
            _ if !self.in_table => {}
            Event::Start(Tag::TableCell) => self.current.clear(),
            Event::End(TagEnd::TableCell) => self.end_cell(),
            Event::End(TagEnd::TableHead) | Event::End(TagEnd::TableRow) => self.write_row(),
            Event::Text(text) | Event::Code(text) => self.current.push_str(&text),
            Event::SoftBreak | Event::HardBreak => self.current.push(' '),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_become_a_table_with_the_header_width() {
        let mut reader = CsvReader::new("a,b\n1,\"x\ny\",extra\n2\n".as_bytes());
        let mut events: Vec<Event<'static>> = Vec::new();
        let notes = emit_table(&mut reader, &mut events).unwrap();
        assert_eq!(
            notes,
            TableNotes {
                trimmed_rows: 1,
                folded_cells: 1
            }
        );
        let texts: Vec<String> = events
            .iter()
            .filter_map(|event| match event {
                Event::Text(text) => Some(text.to_string()),
                _ => None,
            })
            .collect();
        assert_eq!(texts, vec!["a", "b", "1", "x y", "2"]);
        let cells = events
            .iter()
            .filter(|event| **event == Event::Start(Tag::TableCell))
            .count();
        assert_eq!(cells, 6);
    }

    #[test]
    fn the_first_table_comes_out_as_records() {
        let text = "# Title\n\n| a | b |\n|---|---|\n| 1 | **x** y |\n\n| c |\n|---|\n| 2 |\n";
        let mut bytes = Vec::new();
        {
            let mut rows = TableRows::new(&mut bytes, b',');
            crate::io::markdown::parse_into(
                text,
                crate::io::markdown::Options::default(),
                &mut rows,
            );
            assert!(rows.found_table());
            rows.finish().unwrap();
        }
        assert_eq!(String::from_utf8(bytes).unwrap(), "a,b\n1,x y\n");
    }
}
