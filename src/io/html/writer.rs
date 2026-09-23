//! Renders Markdown events to HTML in the form the CommonMark and GFM
//! specifications expect, byte for byte. Footnotes follow cmark-gfm.

use std::fmt::Write as _;

use crate::io::markdown::{Alignment, CodeBlockKind, Event, Tag, TagEnd};
use crate::io::scan::find_html_special;

/// Appends `text` to `out` with `&`, `<`, `>`, and `"` escaped.
pub fn escape_html(out: &mut String, text: &str) {
    let mut remaining = text;
    while let Some(index) = find_html_special(remaining.as_bytes()) {
        out.push_str(&remaining[..index]);
        let byte = remaining.as_bytes()[index];
        match byte {
            b'&' => out.push_str("&amp;"),
            b'<' => out.push_str("&lt;"),
            b'>' => out.push_str("&gt;"),
            _ => out.push_str("&quot;"),
        }
        remaining = &remaining[index + 1..];
    }
    out.push_str(remaining);
}

/// Appends a URL to `out`, percent-encoding bytes outside the safe set the
/// way cmark does, and entity-escaping `&` and `'`.
pub fn escape_href(out: &mut String, url: &str) {
    for byte in url.bytes() {
        let is_safe = byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_'
                    | b'.'
                    | b'+'
                    | b'!'
                    | b'*'
                    | b'('
                    | b')'
                    | b','
                    | b'%'
                    | b'#'
                    | b'@'
                    | b'?'
                    | b'='
                    | b';'
                    | b':'
                    | b'/'
                    | b'$'
                    | b'~'
            );
        if is_safe {
            out.push(byte as char);
        } else if byte == b'&' {
            out.push_str("&amp;");
        } else if byte == b'\'' {
            out.push_str("&#x27;");
        } else {
            let _ = write!(out, "%{byte:02X}");
        }
    }
}

/// Bytes buffered before `write_html` hands a chunk to its sink.
const FLUSH_THRESHOLD: usize = 64 * 1024;

/// Renders `events` into `out`.
pub fn push_html<'a, I: Iterator<Item = Event<'a>>>(out: &mut String, events: I) {
    let mut writer = HtmlWriter {
        out,
        sink: None,
        flushed_at_line_start: true,
        table_alignments: Vec::new(),
        table_column: 0,
        in_table_head: false,
        table_body_open: false,
        containers: Vec::new(),
        image_depth: 0,
        pending_image_title: String::new(),
        footnotes: Vec::new(),
        capture: None,
    };
    for event in events {
        writer.event(event);
    }
    writer.finish();
}

/// Renders `events` straight to `sink`, flushing every 64 KiB, so the
/// whole document is never held as one string.
pub fn write_html<'a, I: Iterator<Item = Event<'a>>>(
    sink: &mut dyn std::io::Write,
    events: I,
) -> std::io::Result<()> {
    let mut buffer = String::with_capacity(FLUSH_THRESHOLD);
    let mut writer = HtmlWriter {
        out: &mut buffer,
        sink: Some(sink),
        flushed_at_line_start: true,
        table_alignments: Vec::new(),
        table_column: 0,
        in_table_head: false,
        table_body_open: false,
        containers: Vec::new(),
        image_depth: 0,
        pending_image_title: String::new(),
        footnotes: Vec::new(),
        capture: None,
    };
    for event in events {
        writer.event(event);
        writer.flush_if_full()?;
    }
    writer.finish();
    writer.flush_all()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Container {
    List { tight: bool },
    Item { tight: bool },
    Other,
}

struct Footnote {
    label: String,
    /// Number of references seen so far; the footnote's number is its
    /// position in this list plus one, assigned at first reference.
    reference_count: usize,
    /// Rendered body, present once its definition has been seen.
    body: Option<String>,
}

struct HtmlWriter<'o> {
    out: &'o mut String,
    /// Present when streaming; `out` is then a chunk buffer.
    sink: Option<&'o mut dyn std::io::Write>,
    /// Whether the last flushed chunk ended with a newline (or nothing has
    /// been flushed yet).
    flushed_at_line_start: bool,
    table_alignments: Vec<Alignment>,
    table_column: usize,
    in_table_head: bool,
    table_body_open: bool,
    /// Open containers, innermost last. A paragraph drops its `<p>` only when
    /// its direct parent is an item of a tight list.
    containers: Vec<Container>,
    image_depth: usize,
    pending_image_title: String,
    footnotes: Vec<Footnote>,
    /// While rendering a footnote definition: the label and the main output
    /// swapped out so the body lands in a separate buffer.
    capture: Option<(String, String)>,
}

impl HtmlWriter<'_> {
    /// Ensures the output ends with a newline, the way cmark separates
    /// block-level tags.
    fn cr(&mut self) {
        if self.out.is_empty() {
            if self.sink.is_some() && !self.flushed_at_line_start {
                self.out.push('\n');
            }
            return;
        }
        if !self.out.ends_with('\n') {
            self.out.push('\n');
        }
    }

    /// Hands the buffer to the sink once it is large enough. Never flushes
    /// while a footnote body is being captured, since that text is not
    /// output yet.
    fn flush_if_full(&mut self) -> std::io::Result<()> {
        if self.out.len() < FLUSH_THRESHOLD || self.capture.is_some() {
            return Ok(());
        }
        self.flush_all()
    }

    fn flush_all(&mut self) -> std::io::Result<()> {
        let sink = match self.sink.as_mut() {
            Some(sink) => sink,
            None => return Ok(()),
        };
        if self.out.is_empty() {
            return Ok(());
        }
        self.flushed_at_line_start = self.out.ends_with('\n');
        sink.write_all(self.out.as_bytes())?;
        self.out.clear();
        Ok(())
    }

    fn event(&mut self, event: Event<'_>) {
        if self.image_depth > 0 {
            self.alt_text_event(&event);
            return;
        }
        match event {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(text) => escape_html(self.out, &text),
            Event::Code(code) => {
                self.out.push_str("<code>");
                escape_html(self.out, &code);
                self.out.push_str("</code>");
            }
            Event::Html(html) | Event::InlineHtml(html) => self.out.push_str(&html),
            Event::SoftBreak => self.out.push('\n'),
            Event::HardBreak => self.out.push_str("<br />\n"),
            Event::Rule => {
                self.cr();
                self.out.push_str("<hr />\n");
            }
            Event::FootnoteReference(label) => self.footnote_reference(&label),
            Event::TaskListMarker(checked) => {
                if checked {
                    self.out
                        .push_str("<input checked=\"\" disabled=\"\" type=\"checkbox\"> ");
                } else {
                    self.out
                        .push_str("<input disabled=\"\" type=\"checkbox\"> ");
                }
            }
        }
    }

    /// Inside an image, content is rendered as plain alt text.
    fn alt_text_event(&mut self, event: &Event<'_>) {
        match event {
            Event::Start(Tag::Image { .. }) => self.image_depth += 1,
            Event::End(TagEnd::Image) => {
                self.image_depth -= 1;
                if self.image_depth == 0 {
                    self.out.push('"');
                    if !self.pending_image_title.is_empty() {
                        self.out.push_str(" title=\"");
                        let title = std::mem::take(&mut self.pending_image_title);
                        escape_html(self.out, &title);
                        self.out.push('"');
                    }
                    self.out.push_str(" />");
                }
            }
            Event::Text(text) | Event::Code(text) => escape_html(self.out, text),
            Event::SoftBreak | Event::HardBreak => self.out.push(' '),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                if self.in_tight_list() {
                    return;
                }
                self.cr();
                self.out.push_str("<p>");
            }
            Tag::Heading(level) => {
                self.cr();
                let _ = write!(self.out, "<h{level}>");
            }
            Tag::BlockQuote => {
                self.cr();
                self.out.push_str("<blockquote>\n");
                self.containers.push(Container::Other);
            }
            Tag::CodeBlock(kind) => {
                self.cr();
                self.out.push_str("<pre><code");
                if let CodeBlockKind::Fenced(info) = kind {
                    let language = info.split(|c: char| c.is_whitespace()).next().unwrap_or("");
                    if !language.is_empty() {
                        self.out.push_str(" class=\"language-");
                        escape_html(self.out, language);
                        self.out.push('"');
                    }
                }
                self.out.push('>');
            }
            Tag::HtmlBlock => self.cr(),
            Tag::List { start, tight } => {
                self.cr();
                match start {
                    Some(1) => self.out.push_str("<ol>\n"),
                    Some(number) => {
                        let _ = writeln!(self.out, "<ol start=\"{number}\">");
                    }
                    None => self.out.push_str("<ul>\n"),
                }
                self.containers.push(Container::List { tight });
            }
            Tag::Item => {
                self.cr();
                self.out.push_str("<li>");
                let tight = match self.containers.last() {
                    Some(Container::List { tight }) => *tight,
                    _ => false,
                };
                self.containers.push(Container::Item { tight });
            }
            Tag::FootnoteDefinition(label) => {
                let body = std::mem::take(self.out);
                self.capture = Some((label.into_owned(), body));
                self.containers.push(Container::Other);
            }
            Tag::Table(alignments) => {
                self.table_alignments = alignments;
                self.cr();
                self.out.push_str("<table>\n");
            }
            Tag::TableHead => {
                self.in_table_head = true;
                self.table_column = 0;
                self.out.push_str("<thead>\n<tr>\n");
            }
            Tag::TableRow => {
                self.table_column = 0;
                if !self.table_body_open {
                    self.table_body_open = true;
                    self.out.push_str("<tbody>\n");
                }
                self.out.push_str("<tr>\n");
            }
            Tag::TableCell => {
                self.containers.push(Container::Other);
                let cell = if self.in_table_head { "<th" } else { "<td" };
                self.out.push_str(cell);
                let alignment = self.table_alignments.get(self.table_column).copied();
                match alignment {
                    Some(Alignment::Left) => self.out.push_str(" align=\"left\""),
                    Some(Alignment::Center) => self.out.push_str(" align=\"center\""),
                    Some(Alignment::Right) => self.out.push_str(" align=\"right\""),
                    _ => {}
                }
                self.out.push('>');
            }
            Tag::Emphasis => self.out.push_str("<em>"),
            Tag::Strong => self.out.push_str("<strong>"),
            Tag::Strikethrough => self.out.push_str("<del>"),
            Tag::Link { destination, title } => {
                self.out.push_str("<a href=\"");
                escape_href(self.out, &destination);
                if !title.is_empty() {
                    self.out.push_str("\" title=\"");
                    escape_html(self.out, &title);
                }
                self.out.push_str("\">");
            }
            Tag::Image { destination, title } => {
                self.out.push_str("<img src=\"");
                escape_href(self.out, &destination);
                self.out.push_str("\" alt=\"");
                self.image_depth = 1;
                self.pending_image_title = title.into_owned();
            }
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph => {
                if self.in_tight_list() {
                    return;
                }
                self.out.push_str("</p>\n");
            }
            TagEnd::Heading(level) => {
                let _ = writeln!(self.out, "</h{level}>");
            }
            TagEnd::BlockQuote => {
                self.cr();
                self.out.push_str("</blockquote>\n");
                self.containers.pop();
            }
            TagEnd::CodeBlock => self.out.push_str("</code></pre>\n"),
            TagEnd::HtmlBlock => self.cr(),
            TagEnd::List(ordered) => {
                self.containers.pop();
                self.cr();
                if ordered {
                    self.out.push_str("</ol>\n");
                } else {
                    self.out.push_str("</ul>\n");
                }
            }
            TagEnd::Item => {
                self.containers.pop();
                self.out.push_str("</li>\n");
            }
            TagEnd::FootnoteDefinition => {
                self.containers.pop();
                self.end_footnote_definition();
            }
            TagEnd::Table => {
                if self.table_body_open {
                    self.out.push_str("</tbody>\n");
                    self.table_body_open = false;
                }
                self.out.push_str("</table>\n");
            }
            TagEnd::TableHead => {
                self.in_table_head = false;
                self.out.push_str("</tr>\n</thead>\n");
            }
            TagEnd::TableRow => self.out.push_str("</tr>\n"),
            TagEnd::TableCell => {
                self.containers.pop();
                if self.in_table_head {
                    self.out.push_str("</th>\n");
                } else {
                    self.out.push_str("</td>\n");
                }
                self.table_column += 1;
            }
            TagEnd::Emphasis => self.out.push_str("</em>"),
            TagEnd::Strong => self.out.push_str("</strong>"),
            TagEnd::Strikethrough => self.out.push_str("</del>"),
            TagEnd::Link => self.out.push_str("</a>"),
            TagEnd::Image => {}
        }
    }

    fn in_tight_list(&self) -> bool {
        matches!(
            self.containers.last(),
            Some(Container::Item { tight: true })
        )
    }

    fn footnote_index(&mut self, label: &str) -> usize {
        let existing = self.footnotes.iter().position(|note| note.label == label);
        match existing {
            Some(index) => index,
            None => {
                self.footnotes.push(Footnote {
                    label: label.to_string(),
                    reference_count: 0,
                    body: None,
                });
                self.footnotes.len() - 1
            }
        }
    }

    fn footnote_reference(&mut self, label: &str) {
        let index = self.footnote_index(label);
        self.footnotes[index].reference_count += 1;
        let count = self.footnotes[index].reference_count;
        let number = self.footnote_number(index);
        self.out
            .push_str("<sup class=\"footnote-ref\"><a href=\"#fn-");
        escape_href(self.out, label);
        self.out.push_str("\" id=\"fnref-");
        escape_href(self.out, label);
        if count > 1 {
            let _ = write!(self.out, "-{count}");
        }
        let _ = write!(self.out, "\" data-footnote-ref>{number}</a></sup>");
    }

    /// Footnotes are numbered by order of first reference; definitions seen
    /// before any reference are numbered after all referenced ones.
    fn footnote_number(&self, index: usize) -> usize {
        let referenced_before = self.footnotes[..index]
            .iter()
            .filter(|note| note.reference_count > 0)
            .count();
        referenced_before + 1
    }

    fn end_footnote_definition(&mut self) {
        let (label, main_output) = match self.capture.take() {
            Some(capture) => capture,
            None => return,
        };
        let body = std::mem::replace(self.out, main_output);
        let index = self.footnote_index(&label);
        self.footnotes[index].body = Some(body);
    }

    fn finish(&mut self) {
        let mut referenced: Vec<usize> = (0..self.footnotes.len())
            .filter(|index| self.footnotes[*index].reference_count > 0)
            .filter(|index| self.footnotes[*index].body.is_some())
            .collect();
        if referenced.is_empty() {
            return;
        }
        referenced.sort_by_key(|index| self.footnote_number(*index));
        self.out
            .push_str("<section class=\"footnotes\" data-footnotes>\n<ol>\n");
        for index in referenced {
            let number = self.footnote_number(index);
            let label = self.footnotes[index].label.clone();
            let body = self.footnotes[index].body.clone().unwrap_or_default();
            let references = self.footnotes[index].reference_count;
            let mut backrefs = String::new();
            for count in 1..=references {
                if count > 1 {
                    backrefs.push(' ');
                }
                backrefs.push_str("<a href=\"#fnref-");
                escape_href(&mut backrefs, &label);
                let idx = if count > 1 {
                    format!("{number}-{count}")
                } else {
                    number.to_string()
                };
                if count > 1 {
                    let _ = write!(backrefs, "-{count}");
                }
                let _ = write!(
                    backrefs,
                    "\" class=\"footnote-backref\" data-footnote-backref data-footnote-backref-idx=\"{idx}\" aria-label=\"Back to reference {idx}\">↩"
                );
                if count > 1 {
                    let _ = write!(backrefs, "<sup class=\"footnote-ref\">{count}</sup>");
                }
                backrefs.push_str("</a>");
            }
            self.out.push_str("<li id=\"fn-");
            escape_href(self.out, &label);
            self.out.push_str("\">\n");
            if let Some(stripped) = body.strip_suffix("</p>\n") {
                self.out.push_str(stripped);
                self.out.push(' ');
                self.out.push_str(&backrefs);
                self.out.push_str("</p>\n");
            } else {
                self.out.push_str(&body);
                self.out.push_str(&backrefs);
                self.out.push('\n');
            }
            self.out.push_str("</li>\n");
        }
        self.out.push_str("</ol>\n</section>\n");
    }
}
