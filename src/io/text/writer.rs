//! Renders Markdown events as plain text meant to be read as is: no
//! markup, structure kept with indentation and list markers, links shown
//! as `text (url)`, tables as aligned columns, footnotes numbered and
//! listed at the end.

use std::io;

use crate::io::markdown::{Event, EventSink, Tag, TagEnd};

const FLUSH_THRESHOLD: usize = 64 * 1024;

/// Renders `events` into `out`.
pub fn push_text<'a, I: Iterator<Item = Event<'a>>>(out: &mut String, events: I) {
    let mut writer = TextWriter::new(std::mem::take(out), None);
    for event in events {
        writer.event(event);
    }
    writer.render_footnotes();
    *out = writer.out;
}

struct Container {
    indent: String,
    /// Written instead of `indent` on the first line inside, then cleared.
    marker: Option<String>,
    has_block: bool,
    /// Blocks inside are separated by a blank line.
    loose: bool,
    /// Number of the next item, for an ordered list container.
    next_number: Option<u64>,
    is_list: bool,
}

struct Table {
    rows: Vec<Vec<String>>,
    /// Text written before the table started, put back when it ends.
    outer: String,
}

struct Footnote {
    label: String,
    number: usize,
    body: Option<String>,
}

pub struct TextWriter<'o> {
    out: String,
    sink: Option<&'o mut dyn io::Write>,
    error: Option<io::Error>,
    containers: Vec<Container>,
    at_line_start: bool,
    /// Byte offset in `out` where each open link's text starts, with its
    /// destination, innermost last.
    links: Vec<(usize, String)>,
    image_depth: usize,
    table: Option<Table>,
    in_html_block: bool,
    footnotes: Vec<Footnote>,
    /// While rendering a footnote definition: its label and the main
    /// output swapped out so the body lands in its own buffer.
    capture: Option<(String, String)>,
}

impl<'o> TextWriter<'o> {
    /// A writer that flushes to `sink` as it goes.
    pub fn streaming(sink: &'o mut dyn io::Write) -> TextWriter<'o> {
        TextWriter::new(String::with_capacity(FLUSH_THRESHOLD), Some(sink))
    }

    fn new(out: String, sink: Option<&'o mut dyn io::Write>) -> TextWriter<'o> {
        TextWriter {
            out,
            sink,
            error: None,
            containers: vec![Container {
                indent: String::new(),
                marker: None,
                has_block: false,
                loose: true,
                next_number: None,
                is_list: false,
            }],
            at_line_start: true,
            links: Vec::new(),
            image_depth: 0,
            table: None,
            in_html_block: false,
            footnotes: Vec::new(),
            capture: None,
        }
    }

    /// Writes the footnotes and the last chunk, and reports the first I/O
    /// error met along the way.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.render_footnotes();
        self.flush_all()
    }

    fn flush_all(&mut self) -> io::Result<()> {
        let sink = match self.sink.as_mut() {
            Some(sink) => sink,
            None => return Ok(()),
        };
        if self.out.is_empty() {
            return Ok(());
        }
        sink.write_all(self.out.as_bytes())?;
        self.out.clear();
        Ok(())
    }

    /// Flushes at a line end when nothing still refers to buffer offsets.
    fn flush_if_full(&mut self) {
        let safe = self.links.is_empty() && self.table.is_none() && self.capture.is_none();
        if !safe || self.out.len() < FLUSH_THRESHOLD || self.error.is_some() {
            return;
        }
        if let Err(error) = self.flush_all() {
            self.error = Some(error);
        }
    }

    // ----- lines and blocks -----

    fn begin_content(&mut self) {
        if !self.at_line_start {
            return;
        }
        self.at_line_start = false;
        for container in &mut self.containers {
            match container.marker.take() {
                Some(marker) => self.out.push_str(&marker),
                None => self.out.push_str(&container.indent),
            }
        }
    }

    fn end_line(&mut self) {
        self.out.push('\n');
        self.at_line_start = true;
        self.flush_if_full();
    }

    fn close_open_line(&mut self) {
        if !self.at_line_start {
            self.end_line();
        }
    }

    fn begin_block(&mut self) {
        let parent = match self.containers.last_mut() {
            Some(parent) => parent,
            None => return,
        };
        let needs_separator = parent.has_block;
        let loose = parent.loose;
        parent.has_block = true;
        if needs_separator {
            self.close_open_line();
            if loose {
                self.end_line();
            }
        }
    }

    fn push_container(
        &mut self,
        indent: String,
        marker: Option<String>,
        loose: bool,
        is_list: bool,
        next_number: Option<u64>,
    ) {
        self.containers.push(Container {
            indent,
            marker,
            has_block: false,
            loose,
            next_number,
            is_list,
        });
    }

    fn pop_container(&mut self) {
        let is_empty = self
            .containers
            .last()
            .is_some_and(|container| !container.has_block && container.marker.is_some());
        if is_empty {
            self.begin_content();
            let trimmed_len = self.out.trim_end_matches(' ').len();
            self.out.truncate(trimmed_len);
        }
        self.close_open_line();
        self.containers.pop();
    }

    // ----- events -----

    fn event(&mut self, event: Event<'_>) {
        if self.in_html_block {
            if let Event::End(TagEnd::HtmlBlock) = event {
                self.in_html_block = false;
            }
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) | Event::Code(text) => {
                self.begin_content();
                self.out.push_str(&text);
            }
            Event::Html(_) | Event::InlineHtml(_) => {}
            Event::SoftBreak | Event::HardBreak => {
                if self.table.is_some() || self.image_depth > 0 {
                    self.out.push(' ');
                } else {
                    self.end_line();
                }
            }
            Event::Rule => {
                self.begin_block();
                self.begin_content();
                self.out.push_str("* * *");
                self.end_line();
            }
            Event::FootnoteReference(label) => {
                self.begin_content();
                let number = self.footnote_number(&label);
                self.out.push_str(&format!("[{number}]"));
            }
            Event::TaskListMarker(checked) => {
                self.begin_content();
                self.out.push_str(if checked { "[x] " } else { "[ ] " });
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph | Tag::Heading(_) | Tag::CodeBlock(_) => self.begin_block(),
            Tag::HtmlBlock => self.in_html_block = true,
            Tag::BlockQuote => {
                self.begin_block();
                self.push_container("  ".to_string(), None, true, false, None);
            }
            Tag::List { start, tight } => {
                self.begin_block();
                self.push_container(String::new(), None, !tight, true, start);
            }
            Tag::Item => {
                self.begin_block();
                let marker = self.next_item_marker();
                let indent = " ".repeat(marker.len());
                let loose = self.containers.last().is_some_and(|list| list.loose);
                self.push_container(indent, Some(marker), loose, false, None);
            }
            Tag::FootnoteDefinition(label) => {
                let body = std::mem::take(&mut self.out);
                self.capture = Some((label.into_owned(), body));
                self.push_container(String::new(), None, true, false, None);
                self.at_line_start = true;
            }
            Tag::Table(_) => {
                self.begin_block();
                let outer = std::mem::take(&mut self.out);
                self.table = Some(Table {
                    rows: Vec::new(),
                    outer,
                });
                self.at_line_start = false;
            }
            Tag::TableHead | Tag::TableRow => {
                if let Some(table) = self.table.as_mut() {
                    table.rows.push(Vec::new());
                }
            }
            Tag::TableCell => self.out.clear(),
            Tag::Emphasis | Tag::Strong | Tag::Strikethrough => self.begin_content(),
            Tag::Link { destination, .. } => {
                self.begin_content();
                self.links.push((self.out.len(), destination.into_owned()));
            }
            Tag::Image { .. } => {
                self.begin_content();
                self.image_depth += 1;
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Heading(_) => self.close_open_line(),
            TagEnd::CodeBlock => self.close_open_line(),
            TagEnd::HtmlBlock => {}
            TagEnd::BlockQuote | TagEnd::Item => self.pop_container(),
            TagEnd::List(_) => self.pop_container(),
            TagEnd::FootnoteDefinition => {
                self.close_open_line();
                self.containers.pop();
                self.end_footnote_definition();
            }
            TagEnd::Table => self.render_table(),
            TagEnd::TableHead | TagEnd::TableRow => {}
            TagEnd::TableCell => {
                let cell = std::mem::take(&mut self.out);
                if let Some(row) = self.table.as_mut().and_then(|table| table.rows.last_mut()) {
                    row.push(cell);
                }
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {}
            TagEnd::Link => {
                if let Some((start, destination)) = self.links.pop() {
                    let text = &self.out[start.min(self.out.len())..];
                    let show_destination = !destination.is_empty() && text != destination;
                    if show_destination {
                        self.out.push_str(" (");
                        self.out.push_str(&destination);
                        self.out.push(')');
                    }
                }
            }
            TagEnd::Image => self.image_depth = self.image_depth.saturating_sub(1),
        }
    }

    fn next_item_marker(&mut self) -> String {
        let list = self
            .containers
            .last_mut()
            .filter(|container| container.is_list);
        match list.and_then(|list| list.next_number.as_mut()) {
            Some(number) => {
                let marker = format!("{number}. ");
                *number = number.wrapping_add(1);
                marker
            }
            None => "- ".to_string(),
        }
    }

    /// Pads every column to its widest cell and separates columns with two
    /// spaces; a line of dashes sits under the header row.
    fn render_table(&mut self) {
        let table = match self.table.take() {
            Some(table) => table,
            None => return,
        };
        self.out = table.outer;
        self.at_line_start = true;
        let column_count = table.rows.iter().map(Vec::len).max().unwrap_or(0);
        let mut widths = vec![0usize; column_count];
        for row in &table.rows {
            for (index, cell) in row.iter().enumerate() {
                widths[index] = widths[index].max(cell.chars().count());
            }
        }
        for (row_index, row) in table.rows.iter().enumerate() {
            self.begin_content();
            self.write_table_row(row, &widths);
            self.end_line();
            if row_index == 0 {
                self.begin_content();
                let mut line = String::new();
                for (index, width) in widths.iter().enumerate() {
                    if index > 0 {
                        line.push_str("  ");
                    }
                    line.push_str(&"-".repeat((*width).max(1)));
                }
                self.out.push_str(&line);
                self.end_line();
            }
        }
    }

    fn write_table_row(&mut self, row: &[String], widths: &[usize]) {
        let last = row.len().saturating_sub(1);
        for (index, cell) in row.iter().enumerate() {
            if index > 0 {
                self.out.push_str("  ");
            }
            self.out.push_str(cell);
            if index < last {
                let padding = widths[index].saturating_sub(cell.chars().count());
                for _ in 0..padding {
                    self.out.push(' ');
                }
            }
        }
    }

    // ----- footnotes -----

    fn footnote_number(&mut self, label: &str) -> usize {
        let existing = self.footnotes.iter().position(|note| note.label == label);
        let index = match existing {
            Some(index) => index,
            None => {
                let number = self.footnotes.iter().filter(|note| note.number > 0).count() + 1;
                self.footnotes.push(Footnote {
                    label: label.to_string(),
                    number,
                    body: None,
                });
                self.footnotes.len() - 1
            }
        };
        if self.footnotes[index].number == 0 {
            let number = self.footnotes.iter().filter(|note| note.number > 0).count() + 1;
            self.footnotes[index].number = number;
        }
        self.footnotes[index].number
    }

    fn end_footnote_definition(&mut self) {
        let (label, main_output) = match self.capture.take() {
            Some(capture) => capture,
            None => return,
        };
        let body = std::mem::replace(&mut self.out, main_output);
        self.at_line_start = self.out.is_empty() || self.out.ends_with('\n');
        match self.footnotes.iter_mut().find(|note| note.label == label) {
            Some(note) => note.body = Some(body),
            None => self.footnotes.push(Footnote {
                label,
                number: 0,
                body: Some(body),
            }),
        }
    }

    /// Referenced footnotes, in number order, each as `[n] body`.
    fn render_footnotes(&mut self) {
        let mut referenced: Vec<usize> = (0..self.footnotes.len())
            .filter(|index| self.footnotes[*index].number > 0)
            .filter(|index| self.footnotes[*index].body.is_some())
            .collect();
        if referenced.is_empty() {
            return;
        }
        referenced.sort_by_key(|index| self.footnotes[*index].number);
        self.close_open_line();
        self.end_line();
        for index in referenced {
            let number = self.footnotes[index].number;
            let body = self.footnotes[index].body.take().unwrap_or_default();
            let marker = format!("[{number}] ");
            let indent = " ".repeat(marker.len());
            for (line_index, line) in body.trim_end_matches('\n').split('\n').enumerate() {
                if line_index == 0 {
                    self.out.push_str(&marker);
                } else if !line.is_empty() {
                    self.out.push_str(&indent);
                }
                self.out.push_str(line);
                self.out.push('\n');
            }
        }
        self.at_line_start = true;
    }
}

impl<'a> EventSink<'a> for TextWriter<'_> {
    fn event(&mut self, event: Event<'a>) {
        self.transient(event);
    }

    fn transient(&mut self, event: Event<'_>) {
        if self.error.is_some() {
            return;
        }
        TextWriter::event(self, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::markdown::Parser;

    fn render(source: &str) -> String {
        let mut out = String::new();
        push_text(&mut out, Parser::new(source));
        out
    }

    #[test]
    fn drops_markup_and_keeps_structure() {
        let out = render(
            "# Title\n\nSome *text* with a [link](https://x.y).\n\n- one\n- two\n  - nested\n\n1. a\n2. b\n",
        );
        assert_eq!(
            out,
            "Title\n\nSome text with a link (https://x.y).\n\n- one\n- two\n  - nested\n\n1. a\n2. b\n"
        );
    }

    #[test]
    fn tables_align_columns() {
        let out = render("| a | bbb |\n|---|---|\n| cc | d |\n");
        assert_eq!(out, "a   bbb\n--  ---\ncc  d\n");
    }

    #[test]
    fn footnotes_are_numbered_and_listed() {
        let out = render("Text[^n].\n\n[^n]: The note.\n");
        assert_eq!(out, "Text[1].\n\n[1] The note.\n");
    }
}
