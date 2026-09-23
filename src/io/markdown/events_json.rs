//! The event stream as JSON, and back. One object per event in a single
//! array; the first key names the event and any other keys are its
//! attributes. Lossless with respect to the events, so Markdown -> JSON ->
//! Markdown round-trips through the writer.
//!
//! ```text
//! {"start":"paragraph"}                      {"end":"paragraph"}
//! {"start":"heading","level":2}              {"end":"heading","level":2}
//! {"start":"list","first":null,"tight":true} {"end":"list","ordered":false}
//! {"start":"code_block","fenced":true,"info":"rust"}
//! {"start":"footnote_definition","label":"1"}
//! {"start":"table","alignments":["none","left","center","right"]}
//! {"start":"link","destination":"/x","title":""}   (also image)
//! {"text":"hello"}   (also code, html, inline_html)
//! {"break":"soft"}   {"break":"hard"}   {"rule":true}
//! {"footnote_reference":"1"}   {"task_list_marker":true}
//! ```
//!
//! Start and end tags: paragraph, heading, block_quote, code_block,
//! html_block, list, item, footnote_definition, table, table_head,
//! table_row, table_cell, emphasis, strong, strikethrough, link, image.

use std::borrow::Cow;
use std::io::{self, Read, Write};

use super::{Alignment, CodeBlockKind, Event, EventSink, Tag, TagEnd};
use crate::io::json::{JsonError, JsonTokenizer, JsonWriter, PreparedKey, Token};

// ----- writing -----

/// The keys, escaped once.
struct Keys {
    start: PreparedKey,
    end: PreparedKey,
    text: PreparedKey,
    code: PreparedKey,
    html: PreparedKey,
    inline_html: PreparedKey,
    line_break: PreparedKey,
    rule: PreparedKey,
    footnote_reference: PreparedKey,
    task_list_marker: PreparedKey,
    level: PreparedKey,
    label: PreparedKey,
    info: PreparedKey,
    fenced: PreparedKey,
    first: PreparedKey,
    tight: PreparedKey,
    ordered: PreparedKey,
    alignments: PreparedKey,
    destination: PreparedKey,
    title: PreparedKey,
}

impl Keys {
    fn new() -> Keys {
        let prepare = JsonWriter::<Vec<u8>>::prepare_key;
        Keys {
            start: prepare("start"),
            end: prepare("end"),
            text: prepare("text"),
            code: prepare("code"),
            html: prepare("html"),
            inline_html: prepare("inline_html"),
            line_break: prepare("break"),
            rule: prepare("rule"),
            footnote_reference: prepare("footnote_reference"),
            task_list_marker: prepare("task_list_marker"),
            level: prepare("level"),
            label: prepare("label"),
            info: prepare("info"),
            fenced: prepare("fenced"),
            first: prepare("first"),
            tight: prepare("tight"),
            ordered: prepare("ordered"),
            alignments: prepare("alignments"),
            destination: prepare("destination"),
            title: prepare("title"),
        }
    }
}

/// Writes events as a JSON array to `sink`.
pub struct JsonEventWriter<W: Write> {
    json: JsonWriter<W>,
    keys: Keys,
    error: Option<io::Error>,
}

impl<W: Write> JsonEventWriter<W> {
    pub fn new(sink: W) -> JsonEventWriter<W> {
        let mut json = JsonWriter::new(sink);
        let error = json.begin_array().err();
        JsonEventWriter {
            json,
            keys: Keys::new(),
            error,
        }
    }

    /// Closes the array and flushes; reports the first I/O error met.
    pub fn finish(mut self) -> io::Result<()> {
        if let Some(error) = self.error.take() {
            return Err(error);
        }
        self.json.end_array()?;
        self.json.flush()
    }

    fn write(&mut self, event: Event<'_>) -> io::Result<()> {
        let json = &mut self.json;
        let keys = &self.keys;
        json.begin_object()?;
        match event {
            Event::Start(tag) => {
                json.prepared_key(&keys.start)?;
                write_tag(json, keys, tag)?;
            }
            Event::End(tag) => {
                json.prepared_key(&keys.end)?;
                write_tag_end(json, keys, tag)?;
            }
            Event::Text(text) => text_event(json, &keys.text, &text)?,
            Event::Code(text) => text_event(json, &keys.code, &text)?,
            Event::Html(text) => text_event(json, &keys.html, &text)?,
            Event::InlineHtml(text) => text_event(json, &keys.inline_html, &text)?,
            Event::SoftBreak => {
                json.prepared_key(&keys.line_break)?;
                json.raw("\"soft\"")?;
            }
            Event::HardBreak => {
                json.prepared_key(&keys.line_break)?;
                json.raw("\"hard\"")?;
            }
            Event::Rule => {
                json.prepared_key(&keys.rule)?;
                json.raw("true")?;
            }
            Event::FootnoteReference(label) => {
                json.prepared_key(&keys.footnote_reference)?;
                json.string(&label)?;
            }
            Event::TaskListMarker(checked) => {
                json.prepared_key(&keys.task_list_marker)?;
                json.raw(bool_text(checked))?;
            }
        }
        json.end_object()
    }
}

fn bool_text(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn text_event<W: Write>(json: &mut JsonWriter<W>, key: &PreparedKey, text: &str) -> io::Result<()> {
    json.prepared_key(key)?;
    json.string(text)
}

fn write_tag<W: Write>(json: &mut JsonWriter<W>, keys: &Keys, tag: Tag<'_>) -> io::Result<()> {
    match tag {
        Tag::Paragraph => json.raw("\"paragraph\""),
        Tag::Heading(level) => {
            json.raw("\"heading\"")?;
            json.prepared_key(&keys.level)?;
            json.raw(&level.to_string())
        }
        Tag::BlockQuote => json.raw("\"block_quote\""),
        Tag::CodeBlock(kind) => {
            json.raw("\"code_block\"")?;
            json.prepared_key(&keys.fenced)?;
            match kind {
                CodeBlockKind::Indented => json.raw("false"),
                CodeBlockKind::Fenced(info) => {
                    json.raw("true")?;
                    json.prepared_key(&keys.info)?;
                    json.string(&info)
                }
            }
        }
        Tag::HtmlBlock => json.raw("\"html_block\""),
        Tag::List { start, tight } => {
            json.raw("\"list\"")?;
            json.prepared_key(&keys.first)?;
            match start {
                Some(number) => json.raw(&number.to_string())?,
                None => json.null()?,
            }
            json.prepared_key(&keys.tight)?;
            json.raw(bool_text(tight))
        }
        Tag::Item => json.raw("\"item\""),
        Tag::FootnoteDefinition(label) => {
            json.raw("\"footnote_definition\"")?;
            json.prepared_key(&keys.label)?;
            json.string(&label)
        }
        Tag::Table(alignments) => {
            json.raw("\"table\"")?;
            json.prepared_key(&keys.alignments)?;
            json.begin_array()?;
            for alignment in alignments {
                json.raw(alignment_name(alignment))?;
            }
            json.end_array()
        }
        Tag::TableHead => json.raw("\"table_head\""),
        Tag::TableRow => json.raw("\"table_row\""),
        Tag::TableCell => json.raw("\"table_cell\""),
        Tag::Emphasis => json.raw("\"emphasis\""),
        Tag::Strong => json.raw("\"strong\""),
        Tag::Strikethrough => json.raw("\"strikethrough\""),
        Tag::Link { destination, title } => link_tag(json, keys, "\"link\"", &destination, &title),
        Tag::Image { destination, title } => {
            link_tag(json, keys, "\"image\"", &destination, &title)
        }
    }
}

fn link_tag<W: Write>(
    json: &mut JsonWriter<W>,
    keys: &Keys,
    kind: &str,
    destination: &str,
    title: &str,
) -> io::Result<()> {
    json.raw(kind)?;
    json.prepared_key(&keys.destination)?;
    json.string(destination)?;
    json.prepared_key(&keys.title)?;
    json.string(title)
}

fn write_tag_end<W: Write>(json: &mut JsonWriter<W>, keys: &Keys, tag: TagEnd) -> io::Result<()> {
    match tag {
        TagEnd::Paragraph => json.raw("\"paragraph\""),
        TagEnd::Heading(level) => {
            json.raw("\"heading\"")?;
            json.prepared_key(&keys.level)?;
            json.raw(&level.to_string())
        }
        TagEnd::BlockQuote => json.raw("\"block_quote\""),
        TagEnd::CodeBlock => json.raw("\"code_block\""),
        TagEnd::HtmlBlock => json.raw("\"html_block\""),
        TagEnd::List(ordered) => {
            json.raw("\"list\"")?;
            json.prepared_key(&keys.ordered)?;
            json.raw(bool_text(ordered))
        }
        TagEnd::Item => json.raw("\"item\""),
        TagEnd::FootnoteDefinition => json.raw("\"footnote_definition\""),
        TagEnd::Table => json.raw("\"table\""),
        TagEnd::TableHead => json.raw("\"table_head\""),
        TagEnd::TableRow => json.raw("\"table_row\""),
        TagEnd::TableCell => json.raw("\"table_cell\""),
        TagEnd::Emphasis => json.raw("\"emphasis\""),
        TagEnd::Strong => json.raw("\"strong\""),
        TagEnd::Strikethrough => json.raw("\"strikethrough\""),
        TagEnd::Link => json.raw("\"link\""),
        TagEnd::Image => json.raw("\"image\""),
    }
}

/// Already quoted.
fn alignment_name(alignment: Alignment) -> &'static str {
    match alignment {
        Alignment::None => "\"none\"",
        Alignment::Left => "\"left\"",
        Alignment::Center => "\"center\"",
        Alignment::Right => "\"right\"",
    }
}

impl<'a, W: Write> EventSink<'a> for JsonEventWriter<W> {
    fn event(&mut self, event: Event<'a>) {
        self.transient(event);
    }

    fn transient(&mut self, event: Event<'_>) {
        if self.error.is_some() {
            return;
        }
        if let Err(error) = self.write(event) {
            self.error = Some(error);
        }
    }
}

// ----- reading -----

/// Which event an object describes, from its naming key.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Missing,
    Start,
    End,
    Text,
    Code,
    Html,
    InlineHtml,
    Break,
    Rule,
    FootnoteReference,
    TaskListMarker,
}

/// Fields of one event object, collected before the event is built. Reused
/// from event to event so the strings keep their capacity; the event
/// borrows them and is handed to the sink as transient.
struct Fields {
    kind: Kind,
    /// The tag name of a start or end event, the text of a text-like
    /// event, `soft` or `hard` for a break, or a footnote label.
    value: String,
    label: String,
    info: String,
    destination: String,
    title: String,
    level: Option<u8>,
    first: Option<u64>,
    tight: Option<bool>,
    ordered: Option<bool>,
    fenced: Option<bool>,
    checked: bool,
    alignments: Vec<Alignment>,
}

impl Fields {
    fn new() -> Fields {
        Fields {
            kind: Kind::Missing,
            value: String::new(),
            label: String::new(),
            info: String::new(),
            destination: String::new(),
            title: String::new(),
            level: None,
            first: None,
            tight: None,
            ordered: None,
            fenced: None,
            checked: false,
            alignments: Vec::new(),
        }
    }

    fn clear(&mut self) {
        self.kind = Kind::Missing;
        self.value.clear();
        self.label.clear();
        self.info.clear();
        self.destination.clear();
        self.title.clear();
        self.level = None;
        self.first = None;
        self.tight = None;
        self.ordered = None;
        self.fenced = None;
        self.checked = false;
        self.alignments.clear();
    }
}

/// The keys an event object may carry: one naming the event, the rest its
/// attributes.
enum Key {
    Names(Kind),
    Label,
    Info,
    Destination,
    Title,
    Level,
    First,
    Tight,
    Ordered,
    Fenced,
    Alignments,
    Unknown,
}

impl Key {
    fn parse(name: &str) -> Key {
        match name {
            "start" => Key::Names(Kind::Start),
            "end" => Key::Names(Kind::End),
            "text" => Key::Names(Kind::Text),
            "code" => Key::Names(Kind::Code),
            "html" => Key::Names(Kind::Html),
            "inline_html" => Key::Names(Kind::InlineHtml),
            "break" => Key::Names(Kind::Break),
            "rule" => Key::Names(Kind::Rule),
            "footnote_reference" => Key::Names(Kind::FootnoteReference),
            "task_list_marker" => Key::Names(Kind::TaskListMarker),
            "label" => Key::Label,
            "info" => Key::Info,
            "destination" => Key::Destination,
            "title" => Key::Title,
            "level" => Key::Level,
            "first" => Key::First,
            "tight" => Key::Tight,
            "ordered" => Key::Ordered,
            "fenced" => Key::Fenced,
            "alignments" => Key::Alignments,
            _ => Key::Unknown,
        }
    }
}

/// Reads an event array from `source` and pushes every event into `sink`.
/// Objects are read one at a time; the whole stream is never held, and
/// each event borrows the reader's buffers.
pub fn read_events_json<R: Read>(
    source: R,
    sink: &mut dyn EventSink<'static>,
) -> Result<(), JsonError> {
    let mut tokens = JsonTokenizer::new(source);
    tokens.expect(Token::BeginArray)?;
    let mut first = true;
    let mut fields = Fields::new();
    loop {
        let token = tokens.next_token()?;
        let token = match token {
            Token::EndArray => break,
            Token::Comma if !first => tokens.next_token()?,
            other => other,
        };
        first = false;
        if token != Token::BeginObject {
            return Err(unexpected(
                &tokens,
                format!("expected an event object, found {}", token.describe()),
            ));
        }
        fields.clear();
        read_fields(&mut tokens, &mut fields)?;
        let event = build_event(&tokens, &fields)?;
        sink.transient(event);
    }
    match tokens.next_token()? {
        Token::End => Ok(()),
        other => Err(unexpected(
            &tokens,
            format!("unexpected {} after the event array", other.describe()),
        )),
    }
}

fn read_fields<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    fields: &mut Fields,
) -> Result<(), JsonError> {
    loop {
        let token = tokens.next_token()?;
        let token = match token {
            Token::EndObject => return Ok(()),
            Token::Comma => tokens.next_token()?,
            other => other,
        };
        if token != Token::String {
            return Err(unexpected(
                tokens,
                format!("expected a key, found {}", token.describe()),
            ));
        }
        let key = Key::parse(tokens.text());
        tokens.expect(Token::Colon)?;
        let value = tokens.next_token()?;
        let is_bool = matches!(value, Token::True | Token::False);
        let flag = value == Token::True;
        match (key, value) {
            (Key::Names(kind), Token::String) if kind != Kind::Rule => {
                fields.kind = kind;
                fields.value.push_str(tokens.text());
            }
            (Key::Names(Kind::Rule), _) if is_bool => fields.kind = Kind::Rule,
            (Key::Names(Kind::TaskListMarker), _) if is_bool => {
                fields.kind = Kind::TaskListMarker;
                fields.checked = flag;
            }
            (Key::Label, Token::String) => fields.label.push_str(tokens.text()),
            (Key::Info, Token::String) => fields.info.push_str(tokens.text()),
            (Key::Destination, Token::String) => fields.destination.push_str(tokens.text()),
            (Key::Title, Token::String) => fields.title.push_str(tokens.text()),
            (Key::Level, Token::Number) => fields.level = tokens.text().parse().ok(),
            (Key::First, Token::Number) => fields.first = tokens.text().parse().ok(),
            (Key::First, Token::Null) => fields.first = None,
            (Key::Tight, _) if is_bool => fields.tight = Some(flag),
            (Key::Ordered, _) if is_bool => fields.ordered = Some(flag),
            (Key::Fenced, _) if is_bool => fields.fenced = Some(flag),
            (Key::Alignments, Token::BeginArray) => {
                read_alignments(tokens, &mut fields.alignments)?;
            }
            (_, value) => {
                let message = format!("unexpected {} value in an event object", value.describe());
                return Err(unexpected(tokens, message));
            }
        }
    }
}

fn read_alignments<R: Read>(
    tokens: &mut JsonTokenizer<R>,
    alignments: &mut Vec<Alignment>,
) -> Result<(), JsonError> {
    loop {
        let token = tokens.next_token()?;
        let token = match token {
            Token::EndArray => return Ok(()),
            Token::Comma => tokens.next_token()?,
            other => other,
        };
        if token != Token::String {
            return Err(unexpected(tokens, "alignments must be strings".to_string()));
        }
        let alignment = match tokens.text() {
            "none" => Alignment::None,
            "left" => Alignment::Left,
            "center" => Alignment::Center,
            "right" => Alignment::Right,
            other => return Err(unexpected(tokens, format!("unknown alignment {other:?}"))),
        };
        alignments.push(alignment);
    }
}

fn build_event<'f, R: Read>(
    tokens: &JsonTokenizer<R>,
    fields: &'f Fields,
) -> Result<Event<'f>, JsonError> {
    let event = match fields.kind {
        Kind::Start => Event::Start(build_tag(tokens, fields)?),
        Kind::End => Event::End(build_tag_end(tokens, fields)?),
        Kind::Text => Event::Text(Cow::Borrowed(&fields.value)),
        Kind::Code => Event::Code(Cow::Borrowed(&fields.value)),
        Kind::Html => Event::Html(Cow::Borrowed(&fields.value)),
        Kind::InlineHtml => Event::InlineHtml(Cow::Borrowed(&fields.value)),
        Kind::Break => match fields.value.as_str() {
            "soft" => Event::SoftBreak,
            "hard" => Event::HardBreak,
            other => return Err(unexpected(tokens, format!("unknown break {other:?}"))),
        },
        Kind::Rule => Event::Rule,
        Kind::FootnoteReference => Event::FootnoteReference(Cow::Borrowed(&fields.value)),
        Kind::TaskListMarker => Event::TaskListMarker(fields.checked),
        Kind::Missing => {
            return Err(unexpected(
                tokens,
                "event object names no event".to_string(),
            ));
        }
    };
    Ok(event)
}

fn build_tag<'f, R: Read>(
    tokens: &JsonTokenizer<R>,
    fields: &'f Fields,
) -> Result<Tag<'f>, JsonError> {
    let tag = match fields.value.as_str() {
        "paragraph" => Tag::Paragraph,
        "heading" => Tag::Heading(fields.level.unwrap_or(1).clamp(1, 6)),
        "block_quote" => Tag::BlockQuote,
        "code_block" => {
            if fields.fenced.unwrap_or(false) {
                Tag::CodeBlock(CodeBlockKind::Fenced(Cow::Borrowed(&fields.info)))
            } else {
                Tag::CodeBlock(CodeBlockKind::Indented)
            }
        }
        "html_block" => Tag::HtmlBlock,
        "list" => Tag::List {
            start: fields.first,
            tight: fields.tight.unwrap_or(true),
        },
        "item" => Tag::Item,
        "footnote_definition" => Tag::FootnoteDefinition(Cow::Borrowed(&fields.label)),
        "table" => Tag::Table(fields.alignments.clone()),
        "table_head" => Tag::TableHead,
        "table_row" => Tag::TableRow,
        "table_cell" => Tag::TableCell,
        "emphasis" => Tag::Emphasis,
        "strong" => Tag::Strong,
        "strikethrough" => Tag::Strikethrough,
        "link" => Tag::Link {
            destination: Cow::Borrowed(&fields.destination),
            title: Cow::Borrowed(&fields.title),
        },
        "image" => Tag::Image {
            destination: Cow::Borrowed(&fields.destination),
            title: Cow::Borrowed(&fields.title),
        },
        other => return Err(unexpected(tokens, format!("unknown start tag {other:?}"))),
    };
    Ok(tag)
}

fn build_tag_end<R: Read>(tokens: &JsonTokenizer<R>, fields: &Fields) -> Result<TagEnd, JsonError> {
    let tag = match fields.value.as_str() {
        "paragraph" => TagEnd::Paragraph,
        "heading" => TagEnd::Heading(fields.level.unwrap_or(1).clamp(1, 6)),
        "block_quote" => TagEnd::BlockQuote,
        "code_block" => TagEnd::CodeBlock,
        "html_block" => TagEnd::HtmlBlock,
        "list" => TagEnd::List(fields.ordered.unwrap_or(false)),
        "item" => TagEnd::Item,
        "footnote_definition" => TagEnd::FootnoteDefinition,
        "table" => TagEnd::Table,
        "table_head" => TagEnd::TableHead,
        "table_row" => TagEnd::TableRow,
        "table_cell" => TagEnd::TableCell,
        "emphasis" => TagEnd::Emphasis,
        "strong" => TagEnd::Strong,
        "strikethrough" => TagEnd::Strikethrough,
        "link" => TagEnd::Link,
        "image" => TagEnd::Image,
        other => return Err(unexpected(tokens, format!("unknown end tag {other:?}"))),
    };
    Ok(tag)
}

fn unexpected<R: Read>(tokens: &JsonTokenizer<R>, message: String) -> JsonError {
    JsonError::Unexpected {
        location: tokens.location(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::markdown::Parser;

    #[test]
    fn events_survive_a_json_round_trip() {
        let source = "# T\n\n- [x] a *b* [c](/d \"e\")\n\n| x | y |\n|:--|--:|\n| 1 | 2 |\n\nf[^n]\n\n[^n]: g\n\n```rust\nh\n```\n";
        let expected: Vec<Event<'_>> = Parser::new(source).collect();
        let mut writer = JsonEventWriter::new(Vec::new());
        for event in Parser::new(source) {
            writer.event(event.clone());
        }
        let mut json = writer.json;
        json.end_array().unwrap();
        let bytes = json.into_inner().unwrap();
        let mut decoded: Vec<Event<'static>> = Vec::new();
        read_events_json(bytes.as_slice(), &mut decoded).unwrap();
        assert_eq!(decoded, expected);
    }
}
