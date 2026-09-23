//! Renders Markdown events back to Markdown. The output parses to the same
//! events, so any format that reaches the event stream reaches Markdown.
//!
//! Every construct is written in one canonical form (ATX headings, `-`
//! bullets, `*` emphasis, backtick fences, pipe tables) and text is escaped
//! wherever it could otherwise be read as markup.

use std::io;

use super::{Alignment, CodeBlockKind, Event, EventSink, Tag, TagEnd};

/// Bytes buffered before a chunk goes to the sink.
const FLUSH_THRESHOLD: usize = 64 * 1024;

/// Renders `events` into `out`.
pub fn push_markdown<'a, I: Iterator<Item = Event<'a>>>(out: &mut String, events: I) {
    let mut writer = MarkdownWriter::new(std::mem::take(out), None);
    for event in events {
        writer.event(event);
    }
    writer.close_open_line();
    *out = writer.out;
}

/// What an open container contributes to the lines inside it.
struct Container {
    /// Start of every line inside this container.
    indent: String,
    /// Written instead of `indent` on the first line inside (a list item
    /// marker or a footnote label), then cleared.
    marker: Option<String>,
    /// A block has been written inside, so the next one needs a separator.
    has_block: bool,
    /// Blocks inside are separated by a blank line; false inside a tight
    /// list, where a newline is enough.
    loose: bool,
    /// Present for a list container.
    list: Option<ListState>,
    /// A footnote definition, whose marker cannot share a line with
    /// indented code.
    footnote: bool,
    /// Marker character of a list that just closed directly inside this
    /// container, so a following list of the same kind can use the other
    /// one and stay a separate list.
    last_list_marker: Option<u8>,
}

struct ListState {
    /// Number of the next item, or `None` for a bullet list.
    next_number: Option<u64>,
    /// `-` or `*` for bullets; `.` or `)` for ordered lists.
    marker: u8,
}

/// A code block whose text is collected until it closes, because the fence
/// must be longer than any backtick run inside.
struct CodeBlock {
    fenced: bool,
    info: String,
    content: String,
}

/// Renders events to Markdown. As an [`EventSink`] it consumes events
/// straight from a parser; in streaming mode it flushes to its sink at
/// line ends once the buffer is large enough.
pub struct MarkdownWriter<'o> {
    out: String,
    sink: Option<&'o mut dyn io::Write>,
    error: Option<io::Error>,
    containers: Vec<Container>,
    /// Nothing has been written on the current line yet.
    at_line_start: bool,
    code: Option<CodeBlock>,
    table_alignments: Vec<Alignment>,
    in_table_cell: bool,
    /// While inside a heading: its level and the output swapped out, so
    /// the content can be inspected before the form is chosen.
    heading: Option<(u8, String)>,
    in_heading: bool,
    /// Destination and title of each open link or image, innermost last.
    links: Vec<(String, String)>,
    /// Delimiter character of each open emphasis or strong run, innermost
    /// last, and whether the run sits inside a word. A run opened right
    /// after another uses the other character, so `*_a_*` does not come
    /// back as `**a**`; inside a word only `*` works, and there the parser's
    /// own rules read `***a***` back correctly.
    emphasis_delimiters: Vec<(char, bool)>,
    /// Length of `out` right after the last run opener was written.
    run_opened_at: usize,
    /// When the current line so far is only digits (from a text event
    /// split by an escape), how many; a following `.` or `)` would then
    /// make a list marker.
    line_digits: usize,
}

impl<'o> MarkdownWriter<'o> {
    /// A writer that flushes to `sink` as it goes.
    pub fn streaming(sink: &'o mut dyn io::Write) -> MarkdownWriter<'o> {
        MarkdownWriter::new(String::with_capacity(FLUSH_THRESHOLD), Some(sink))
    }

    fn new(out: String, sink: Option<&'o mut dyn io::Write>) -> MarkdownWriter<'o> {
        MarkdownWriter {
            out,
            sink,
            error: None,
            containers: vec![Container {
                indent: String::new(),
                marker: None,
                has_block: false,
                loose: true,
                list: None,
                footnote: false,
                last_list_marker: None,
            }],
            at_line_start: true,
            code: None,
            table_alignments: Vec::new(),
            in_table_cell: false,
            heading: None,
            in_heading: false,
            links: Vec::new(),
            line_digits: 0,
            emphasis_delimiters: Vec::new(),
            run_opened_at: 0,
        }
    }

    /// Writes the last chunk and reports the first I/O error met.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.close_open_line();
        self.flush_all()
    }

    fn close_open_line(&mut self) {
        if !self.at_line_start {
            self.end_line();
        }
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

    // ----- lines and blocks -----

    /// Writes the container prefixes if the line is fresh.
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
        if self.out.len() >= FLUSH_THRESHOLD && self.error.is_none() {
            if let Err(error) = self.flush_all() {
                self.error = Some(error);
            }
        }
    }

    /// A line with nothing but the prefixes, trailing spaces trimmed.
    fn blank_line(&mut self) {
        let mut line = String::new();
        for container in &self.containers {
            line.push_str(&container.indent);
        }
        let trimmed = line.trim_end();
        self.out.push_str(trimmed);
        self.end_line();
    }

    /// Separates a new block from the previous one in the same container.
    fn begin_block(&mut self, is_list: bool) {
        let parent = match self.containers.last_mut() {
            Some(parent) => parent,
            None => return,
        };
        let needs_separator = parent.has_block;
        let loose = parent.loose;
        parent.has_block = true;
        if !is_list {
            parent.last_list_marker = None;
        }
        if needs_separator {
            self.close_open_line();
            if loose {
                self.blank_line();
            }
        }
    }

    fn push_container(
        &mut self,
        indent: String,
        marker: Option<String>,
        loose: bool,
        list: Option<ListState>,
    ) {
        self.containers.push(Container {
            indent,
            marker,
            has_block: false,
            loose,
            list,
            footnote: false,
            last_list_marker: None,
        });
    }

    /// Closes a container. An empty one still gets its marker line.
    fn pop_container(&mut self) -> Option<Container> {
        let is_empty = self
            .containers
            .last()
            .is_some_and(|container| !container.has_block);
        if is_empty {
            self.begin_content();
            let trimmed_len = self.out.trim_end_matches(' ').len();
            self.out.truncate(trimmed_len);
        }
        self.close_open_line();
        self.containers.pop()
    }

    // ----- events -----

    fn event(&mut self, event: Event<'_>) {
        if let Some(code) = self.code.as_mut() {
            match event {
                Event::Text(text) => code.content.push_str(&text),
                Event::End(TagEnd::CodeBlock) => self.end_code_block(),
                _ => {}
            }
            return;
        }
        if !matches!(event, Event::Text(_)) {
            self.line_digits = 0;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => self.text(&text),
            Event::Code(code) => self.code_span(&code),
            Event::Html(html) => {
                self.begin_content();
                let raw = unfilter_tags(&html);
                self.out.push_str(&raw);
                if raw.ends_with('\n') {
                    self.at_line_start = true;
                }
            }
            Event::InlineHtml(html) => {
                self.begin_content();
                self.out.push_str(&unfilter_tags(&html));
            }
            Event::SoftBreak => self.end_line(),
            Event::HardBreak => {
                self.begin_content();
                self.out.push('\\');
                self.end_line();
            }
            Event::Rule => {
                self.begin_block(false);
                self.begin_content();
                self.out.push_str("***");
                self.end_line();
            }
            Event::FootnoteReference(label) => {
                self.begin_content();
                self.out.push_str("[^");
                self.out.push_str(&label);
                self.out.push(']');
            }
            Event::TaskListMarker(checked) => {
                self.begin_content();
                self.out.push_str(if checked { "[x] " } else { "[ ] " });
            }
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                self.begin_block(false);
            }
            Tag::Heading(level) => {
                self.begin_block(false);
                self.begin_content();
                let outer = std::mem::take(&mut self.out);
                self.heading = Some((level, outer));
                self.in_heading = true;
            }
            Tag::BlockQuote => {
                self.begin_block(false);
                self.push_container("> ".to_string(), None, true, None);
            }
            Tag::CodeBlock(kind) => {
                // Indented code right after a list would be read as part of
                // its last item, so it is fenced there instead.
                let after_list = self
                    .containers
                    .last()
                    .is_some_and(|parent| parent.last_list_marker.is_some());
                self.begin_block(false);
                let (fenced, info) = match kind {
                    CodeBlockKind::Indented => (after_list, String::new()),
                    CodeBlockKind::Fenced(info) => (true, info.into_owned()),
                };
                let marker_pending = self
                    .containers
                    .last()
                    .is_some_and(|container| container.footnote && container.marker.is_some());
                if !fenced && marker_pending {
                    // `[^x]:` followed by code needs the code on its own line.
                    self.begin_content();
                    let trimmed_len = self.out.trim_end_matches(' ').len();
                    self.out.truncate(trimmed_len);
                    self.end_line();
                }
                self.code = Some(CodeBlock {
                    fenced,
                    info,
                    content: String::new(),
                });
            }
            Tag::HtmlBlock => self.begin_block(false),
            Tag::List { start, tight } => {
                self.begin_block(true);
                let previous = self
                    .containers
                    .last()
                    .and_then(|parent| parent.last_list_marker);
                let marker = match (start.is_some(), previous) {
                    (false, Some(b'-')) => b'*',
                    (false, _) => b'-',
                    (true, Some(b'.')) => b')',
                    (true, _) => b'.',
                };
                let list = ListState {
                    next_number: start,
                    marker,
                };
                self.push_container(String::new(), None, !tight, Some(list));
            }
            Tag::Item => {
                self.begin_block(false);
                let marker = self.next_item_marker();
                let indent = " ".repeat(marker.len());
                let loose = self.containers.last().is_some_and(|list| list.loose);
                self.push_container(indent, Some(marker), loose, None);
            }
            Tag::FootnoteDefinition(label) => {
                self.begin_block(false);
                let marker = format!("[^{label}]: ");
                self.push_container("    ".to_string(), Some(marker), true, None);
                if let Some(container) = self.containers.last_mut() {
                    container.footnote = true;
                }
            }
            Tag::Table(alignments) => {
                self.begin_block(false);
                self.table_alignments = alignments;
            }
            Tag::TableHead | Tag::TableRow => {
                self.begin_content();
                self.out.push('|');
            }
            Tag::TableCell => {
                self.out.push(' ');
                self.in_table_cell = true;
            }
            Tag::Emphasis => self.open_run(1),
            Tag::Strong => self.open_run(2),
            Tag::Strikethrough => {
                self.begin_content();
                self.out.push_str("~~");
            }
            Tag::Link { destination, title } => {
                self.begin_content();
                self.out.push('[');
                self.links
                    .push((destination.into_owned(), title.into_owned()));
            }
            Tag::Image { destination, title } => {
                self.begin_content();
                self.out.push_str("![");
                self.links
                    .push((destination.into_owned(), title.into_owned()));
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => self.close_open_line(),
            TagEnd::Heading(_) => self.end_heading(),
            TagEnd::BlockQuote | TagEnd::FootnoteDefinition => {
                self.pop_container();
            }
            TagEnd::CodeBlock => {}
            TagEnd::HtmlBlock => self.close_open_line(),
            TagEnd::List(_) => {
                let list = self.pop_container();
                let marker = list.and_then(|list| list.list).map(|list| list.marker);
                if let Some(parent) = self.containers.last_mut() {
                    parent.last_list_marker = marker;
                }
            }
            TagEnd::Item => {
                self.pop_container();
            }
            TagEnd::Table => {}
            TagEnd::TableHead => {
                self.end_line();
                self.delimiter_row();
            }
            TagEnd::TableRow => self.end_line(),
            TagEnd::TableCell => {
                self.out.push_str(" |");
                self.in_table_cell = false;
            }
            TagEnd::Emphasis => self.close_run(1),
            TagEnd::Strong => self.close_run(2),
            TagEnd::Strikethrough => self.out.push_str("~~"),
            TagEnd::Link | TagEnd::Image => self.end_link(),
        }
    }

    fn open_run(&mut self, count: usize) {
        self.begin_content();
        let adjacent = self.run_opened_at > 0 && self.out.len() == self.run_opened_at;
        let intraword = match self.emphasis_delimiters.last() {
            Some((_, outer_intraword)) if adjacent => *outer_intraword,
            _ => self.out.ends_with(|ch: char| ch.is_alphanumeric()),
        };
        let delimiter = match (adjacent, intraword, self.emphasis_delimiters.last()) {
            (true, false, Some(('*', _))) => '_',
            _ => '*',
        };
        for _ in 0..count {
            self.out.push(delimiter);
        }
        self.emphasis_delimiters.push((delimiter, intraword));
        self.run_opened_at = self.out.len();
    }

    fn close_run(&mut self, count: usize) {
        let (delimiter, _) = self.emphasis_delimiters.pop().unwrap_or(('*', false));
        for _ in 0..count {
            self.out.push(delimiter);
        }
    }

    /// An ATX heading, or a setext one when the content spans lines,
    /// which ATX cannot hold. Levels past two have no setext form, so
    /// there the line breaks become spaces.
    fn end_heading(&mut self) {
        self.in_heading = false;
        let (level, outer) = match self.heading.take() {
            Some(heading) => heading,
            None => return,
        };
        let content = std::mem::replace(&mut self.out, outer);
        let content = content.trim_end_matches('\n');
        let multiline = content.contains('\n');
        if multiline && level <= 2 {
            let mut prefix = String::new();
            for container in &self.containers {
                prefix.push_str(&container.indent);
            }
            for (index, line) in content.split('\n').enumerate() {
                if index > 0 {
                    self.out.push_str(&prefix);
                }
                self.out.push_str(line);
                self.out.push('\n');
            }
            self.out.push_str(&prefix);
            let underline = if level == 1 { "===" } else { "---" };
            self.out.push_str(underline);
            self.end_line();
            return;
        }
        for _ in 0..level {
            self.out.push('#');
        }
        if !content.is_empty() {
            self.out.push(' ');
            let joined = content.replace('\n', " ");
            self.out.push_str(joined.trim_end_matches(' '));
        }
        self.end_line();
    }

    /// Writes `](destination "title")`. A destination with spaces, angle
    /// brackets, or parentheses goes in angle brackets, where only `<`,
    /// `>`, and `\` need escaping.
    fn end_link(&mut self) {
        let (destination, title) = match self.links.pop() {
            Some(link) => link,
            None => return,
        };
        self.out.push_str("](");
        let needs_brackets = destination.is_empty()
            || destination
                .bytes()
                .any(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'<' | b'>' | b'(' | b')'));
        if needs_brackets {
            self.out.push('<');
        }
        for ch in destination.chars() {
            if matches!(ch, '\\' | '<' | '>' | '&' | '(' | ')') {
                self.out.push('\\');
            }
            self.out.push(ch);
        }
        if needs_brackets {
            self.out.push('>');
        }
        if !title.is_empty() {
            self.out.push_str(" \"");
            for ch in title.chars() {
                if matches!(ch, '\\' | '"' | '&') {
                    self.out.push('\\');
                }
                self.out.push(ch);
            }
            self.out.push('"');
        }
        self.out.push(')');
    }

    fn next_item_marker(&mut self) -> String {
        let list = self
            .containers
            .last_mut()
            .and_then(|container| container.list.as_mut());
        match list {
            Some(list) => match list.next_number {
                Some(number) => {
                    list.next_number = Some(number.wrapping_add(1));
                    format!("{number}{} ", list.marker as char)
                }
                None => format!("{} ", list.marker as char),
            },
            None => "- ".to_string(),
        }
    }

    fn delimiter_row(&mut self) {
        self.begin_content();
        self.out.push('|');
        let alignments = std::mem::take(&mut self.table_alignments);
        for alignment in &alignments {
            let cell = match alignment {
                Alignment::None => " --- |",
                Alignment::Left => " :-- |",
                Alignment::Center => " :-: |",
                Alignment::Right => " --: |",
            };
            self.out.push_str(cell);
        }
        self.table_alignments = alignments;
        self.end_line();
    }

    fn end_code_block(&mut self) {
        let code = match self.code.take() {
            Some(code) => code,
            None => return,
        };
        let lines = code.content.strip_suffix('\n').unwrap_or(&code.content);
        if code.fenced {
            let fence = choose_fence(&code.content, &code.info);
            self.begin_content();
            self.out.push_str(&fence);
            self.out.push_str(&code.info);
            self.end_line();
            if !code.content.is_empty() {
                for line in lines.split('\n') {
                    self.code_line(line, "");
                }
            }
            self.begin_content();
            self.out.push_str(&fence);
            self.end_line();
        } else if !code.content.is_empty() {
            for line in lines.split('\n') {
                self.code_line(line, "    ");
            }
        }
    }

    fn code_line(&mut self, line: &str, indent: &str) {
        if line.is_empty() {
            self.blank_line();
            return;
        }
        self.begin_content();
        self.out.push_str(indent);
        self.out.push_str(line);
        self.end_line();
    }

    fn code_span(&mut self, code: &str) {
        self.begin_content();
        let ticks = "`".repeat(longest_run(code, b'`') + 1);
        let needs_padding = code.starts_with('`')
            || code.ends_with('`')
            || (code.starts_with(' ') && code.ends_with(' ') && !code.trim().is_empty());
        self.out.push_str(&ticks);
        if needs_padding {
            self.out.push(' ');
        }
        if self.in_table_cell {
            // GFM splits cells at pipes even inside code spans.
            self.out.push_str(&code.replace('|', "\\|"));
        } else {
            self.out.push_str(code);
        }
        if needs_padding {
            self.out.push(' ');
        }
        self.out.push_str(&ticks);
    }

    // ----- text escaping -----

    fn text(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        let line_fresh = self.at_line_start;
        self.begin_content();
        let bytes = text.as_bytes();
        let mut index = 0usize;
        if line_fresh {
            index = self.escape_line_start(text);
        } else if self.line_digits > 0 && is_marker_delimiter(bytes) {
            self.out.push('\\');
        }
        let all_digits = bytes.iter().all(u8::is_ascii_digit);
        let digits_so_far = if line_fresh { 0 } else { self.line_digits };
        self.line_digits = if all_digits && (line_fresh || digits_so_far > 0) {
            (digits_so_far + bytes.len()).min(10)
        } else {
            0
        };
        while index < bytes.len() {
            let byte = bytes[index];
            if byte >= 0x80 {
                let mut next = index + 1;
                while next < bytes.len() && (bytes[next] & 0xC0) == 0x80 {
                    next += 1;
                }
                self.out.push_str(&text[index..next]);
                index = next;
                continue;
            }
            if byte == b'\n' {
                // A literal newline in text (from `&#10;`) must not end the line.
                self.out.push_str("&#10;");
                index += 1;
                continue;
            }
            let escaped = match byte {
                b'\\' | b'*' | b'_' | b'`' | b'[' | b']' | b'<' | b'~' | b'&' | b'@' => true,
                // A `!` right before a link would make it an image.
                b'!' => index + 1 == bytes.len(),
                b'#' => self.in_heading,
                b'|' => self.in_table_cell,
                b':' => bytes[index + 1..].starts_with(b"//"),
                b'.' => index >= 3 && bytes[index - 3..index].eq_ignore_ascii_case(b"www"),
                _ => false,
            };
            if escaped {
                self.out.push('\\');
            }
            self.out.push(byte as char);
            index += 1;
        }
    }

    /// Escapes whatever at the start of a line would open a block, and
    /// returns how many bytes it consumed.
    fn escape_line_start(&mut self, text: &str) -> usize {
        let bytes = text.as_bytes();
        let first = bytes[0];
        if first == b' ' || first == b'\t' {
            // Leading whitespace would be stripped or start indented code.
            self.out
                .push_str(if first == b' ' { "&#32;" } else { "&#9;" });
            return 1;
        }
        if matches!(
            first,
            b'#' | b'>' | b'-' | b'+' | b'*' | b'=' | b'~' | b'`' | b'_'
        ) {
            self.out.push('\\');
            self.out.push(first as char);
            return 1;
        }
        let digits = bytes
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits > 0 && digits < 10 && is_marker_delimiter(&bytes[digits..]) {
            self.out.push_str(&text[..digits]);
            self.out.push('\\');
            self.out.push(bytes[digits] as char);
            return digits + 1;
        }
        0
    }
}

/// Undoes the GFM tag filter, which the parser applies to raw HTML: a
/// `&lt;` that it put in front of a disallowed tag name becomes `<` again,
/// so the raw HTML re-parses (and re-filters) to the same event.
fn unfilter_tags(html: &str) -> std::borrow::Cow<'_, str> {
    if !html.contains("&lt;") {
        return std::borrow::Cow::Borrowed(html);
    }
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(index) = rest.find("&lt;") {
        out.push_str(&rest[..index]);
        let after = &rest[index + 4..];
        let name = after.trim_start_matches('/');
        let filtered = super::scan::FILTERED_TAGS.iter().any(|tag| {
            name.len() >= tag.len()
                && name[..tag.len()].eq_ignore_ascii_case(tag)
                && !name
                    .as_bytes()
                    .get(tag.len())
                    .is_some_and(u8::is_ascii_alphanumeric)
        });
        out.push_str(if filtered { "<" } else { "&lt;" });
        rest = after;
    }
    out.push_str(rest);
    std::borrow::Cow::Owned(out)
}

/// `.` or `)` followed by a space, a tab, or nothing: what turns a run of
/// digits into an ordered list marker.
fn is_marker_delimiter(bytes: &[u8]) -> bool {
    matches!(bytes.first(), Some(b'.') | Some(b')'))
        && matches!(bytes.get(1), None | Some(b' ') | Some(b'\t'))
}

/// A backtick fence longer than any backtick run in `content`, or a tilde
/// fence when the info string holds a backtick (which a backtick fence
/// forbids).
fn choose_fence(content: &str, info: &str) -> String {
    let fence_char = if info.contains('`') { b'~' } else { b'`' };
    let length = (longest_run(content, fence_char) + 1).max(3);
    (fence_char as char).to_string().repeat(length)
}

fn longest_run(text: &str, needle: u8) -> usize {
    let mut longest = 0usize;
    let mut current = 0usize;
    for byte in text.bytes() {
        if byte == needle {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

impl<'a> EventSink<'a> for MarkdownWriter<'_> {
    fn event(&mut self, event: Event<'a>) {
        self.transient(event);
    }

    fn transient(&mut self, event: Event<'_>) {
        if self.error.is_some() {
            return;
        }
        MarkdownWriter::event(self, event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::markdown::Parser;

    fn round_trip(source: &str) -> String {
        let mut out = String::new();
        push_markdown(&mut out, Parser::new(source));
        out
    }

    #[test]
    fn writes_canonical_forms() {
        let out = round_trip("Title\n=====\n\n+ one\n+ two\n\n1) a\n2) b\n\n~~~\ncode\n~~~\n");
        assert_eq!(
            out,
            "# Title\n\n- one\n- two\n\n1. a\n2. b\n\n```\ncode\n```\n"
        );
    }

    #[test]
    fn escapes_text_that_would_become_markup() {
        let out = round_trip("\\# not a heading\n\n1\\. not a list\n\na \\* b \\_ c\n");
        assert_eq!(
            out,
            "\\# not a heading\n\n1\\. not a list\n\na \\* b \\_ c\n"
        );
    }

    #[test]
    fn keeps_adjacent_lists_apart() {
        let out = round_trip("- a\n\n* b\n");
        assert_eq!(out, "- a\n\n* b\n");
    }

    #[test]
    fn nested_containers_carry_their_prefixes() {
        let out = round_trip("> - one\n>   two\n>\n>   more\n");
        assert_eq!(out, "> - one\n>   two\n>\n>   more\n");
    }
}
