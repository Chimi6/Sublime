//! A Pages package as JSON (`pages-json`): every entry, with each object's
//! header and messages as schema-named trees. Lossless at the object level:
//! reading the JSON back gives trees that re-encode to the original bytes.
//!
//! ```text
//! {"format":"pages-json","version":1,"entries":[
//!   {"stream":"Index/Document.iwa","objects":[
//!     {"info":{"@type":"TSP.ArchiveInfo","identifier":1,"message_infos":[{...}]},
//!      "messages":[{"@type":"TP.DocumentArchive","@id":10000,"stylesheet":1686817,...}]}]},
//!   {"file":"Data/image1.png","base64":"..."}]}
//! ```
//!
//! Field values: numbers, booleans, and strings as expected; references as
//! the object identifier; bytes as base64; repeated fields as arrays; a
//! float that is not finite as `{"float_bits":n}` or `{"double_bits":n}`.
//! Fields without a schema are keyed `"#<number>"` and wrapped by wire
//! type: `{"raw_varint":n}`, `{"raw_fixed32":n}`, `{"raw_fixed64":n}`,
//! `{"raw_bytes":"base64"}`; a known field whose value could not be
//! decoded as its kind carries the same wrappers. In a message object,
//! `@type` and `@id` come before the fields.

use std::io::{self, Read, Write};

use super::package::{Entry, Object, ObjectMessage, Package, Stream};
use super::schema::SCHEMA;
use super::{message_schema, type_name};
use crate::io::base64;
use crate::io::json::{JsonError, JsonTokenizer, JsonWriter, Token};
use crate::io::protobuf::schema::{Field, Kind, MessageRef};
use crate::io::protobuf::tree::{Chain, NONE, Node, Tree, TreeError};

// ----- writing -----

pub fn write_json<W: Write>(package: &Package, sink: W) -> io::Result<()> {
    let mut json = JsonWriter::new(sink);
    json.begin_object()?;
    json.key("format")?;
    json.string("pages-json")?;
    json.key("version")?;
    json.raw("1")?;
    json.key("entries")?;
    json.begin_array()?;
    for entry in &package.entries {
        json.begin_object()?;
        match entry {
            Entry::File { name, bytes } => {
                json.key("file")?;
                json.string(name)?;
                json.key("base64")?;
                json.base64_string(bytes)?;
            }
            Entry::Stream(stream) => {
                json.key("stream")?;
                json.string(&stream.name)?;
                json.key("objects")?;
                json.begin_array()?;
                for object in &stream.objects {
                    write_object(&mut json, &stream.tree, object)?;
                }
                json.end_array()?;
            }
        }
        json.end_object()?;
    }
    json.end_array()?;
    json.end_object()?;
    json.flush()
}

fn write_object<W: Write>(
    json: &mut JsonWriter<W>,
    tree: &Tree,
    object: &Object,
) -> io::Result<()> {
    json.begin_object()?;
    json.key("info")?;
    json.begin_object()?;
    json.key("@type")?;
    json.string("TSP.ArchiveInfo")?;
    write_fields(json, tree, object.info)?;
    json.end_object()?;
    json.key("messages")?;
    json.begin_array()?;
    for message in &object.messages {
        json.begin_object()?;
        json.key("@type")?;
        match type_name(message.message_type) {
            Some(name) => json.string(name)?,
            None => json.raw(&message.message_type.to_string())?,
        }
        json.key("@id")?;
        json.raw(&message.message_type.to_string())?;
        write_fields(json, tree, message.first)?;
        json.end_object()?;
    }
    json.end_array()?;
    json.end_object()
}

/// Consecutive entries of one field become an array when the field is
/// repeated (or unknown and repeated in the data).
fn write_fields<W: Write>(json: &mut JsonWriter<W>, tree: &Tree, first: u32) -> io::Result<()> {
    let mut cursor = first;
    while cursor != NONE {
        let entry = &tree.entries[cursor as usize];
        let field = tree.field(entry);
        // Find the run of this field.
        let mut run_end = entry.next;
        let mut run_length = 1usize;
        while run_end != NONE {
            let candidate = &tree.entries[run_end as usize];
            if candidate.number != entry.number || tree.field(candidate) != field {
                break;
            }
            run_end = candidate.next;
            run_length += 1;
        }
        match field {
            Some(field) => json.key(field.name)?,
            None => json.key(&format!("#{}", entry.number))?,
        }
        let repeated = field.is_some_and(|field| field.repeated) || run_length > 1;
        if repeated {
            json.begin_array()?;
        }
        let mut element = cursor;
        while element != run_end {
            let entry = &tree.entries[element as usize];
            write_node(json, tree, entry.value, field)?;
            element = entry.next;
        }
        if repeated {
            json.end_array()?;
        }
        cursor = run_end;
    }
    Ok(())
}

fn write_node<W: Write>(
    json: &mut JsonWriter<W>,
    tree: &Tree,
    node: Node,
    field: Option<Field>,
) -> io::Result<()> {
    match node {
        Node::Int(value) => json.raw(&value.to_string()),
        Node::Uint(value) => json.raw(&value.to_string()),
        Node::Bool(value) => json.raw(if value { "true" } else { "false" }),
        Node::Fixed32(value) => json.raw(&value.to_string()),
        Node::Fixed64(value) => json.raw(&value.to_string()),
        Node::Float(value) => {
            if value.is_finite() {
                json.raw(&value.to_string())
            } else {
                wrapped(json, "float_bits", &value.to_bits().to_string())
            }
        }
        Node::Double(value) => {
            if value.is_finite() {
                json.raw(&value.to_string())
            } else {
                wrapped(json, "double_bits", &value.to_bits().to_string())
            }
        }
        Node::Str(span) => json.string(tree.str(span)),
        Node::Bytes(span) => json.base64_string(tree.bytes(span)),
        Node::Reference(identifier) => json.raw(&identifier.to_string()),
        Node::Message(first) => {
            json.begin_object()?;
            if let Some(Kind::Message(index)) = field.map(|field| field.kind)
                && let Some(nested) = SCHEMA.message_at(index)
            {
                json.key("@type")?;
                json.string(nested.name())?;
            }
            write_fields(json, tree, first)?;
            json.end_object()
        }
        Node::RawVarint(value) => wrapped(json, "raw_varint", &value.to_string()),
        Node::RawFixed32(value) => wrapped(json, "raw_fixed32", &value.to_string()),
        Node::RawFixed64(value) => wrapped(json, "raw_fixed64", &value.to_string()),
        Node::RawBytes(span) => {
            json.begin_object()?;
            json.key("raw_bytes")?;
            json.base64_string(tree.bytes(span))?;
            json.end_object()
        }
        // Only a document-scope package holds deferred fields; the JSON
        // form decodes everything, so this is the raw form for completeness.
        Node::Deferred(span) => {
            json.begin_object()?;
            json.key("deferred")?;
            json.base64_string(tree.bytes(span))?;
            json.end_object()
        }
    }
}

fn wrapped<W: Write>(json: &mut JsonWriter<W>, key: &str, raw: &str) -> io::Result<()> {
    json.begin_object()?;
    json.key(key)?;
    json.raw(raw)?;
    json.end_object()
}

// ----- reading -----

#[derive(Debug)]
pub enum ReadError {
    Json(JsonError),
    Tree(TreeError),
    /// A structural problem, with where it was met.
    Shape {
        line: u64,
        what: String,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Json(error) => write!(formatter, "{error}"),
            ReadError::Tree(error) => write!(formatter, "{error}"),
            ReadError::Shape { line, what } => write!(formatter, "line {line}: {what}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<JsonError> for ReadError {
    fn from(error: JsonError) -> Self {
        ReadError::Json(error)
    }
}

impl From<TreeError> for ReadError {
    fn from(error: TreeError) -> Self {
        ReadError::Tree(error)
    }
}

struct Reader<R: Read> {
    tokens: JsonTokenizer<R>,
}

impl<R: Read> Reader<R> {
    fn shape(&self, what: impl Into<String>) -> ReadError {
        ReadError::Shape {
            line: self.tokens.location().line,
            what: what.into(),
        }
    }

    fn key(&mut self) -> Result<Option<String>, ReadError> {
        match self.tokens.next_token()? {
            Token::EndObject => Ok(None),
            Token::Comma => self.key_after_comma(),
            Token::String => {
                let key = self.tokens.text().to_string();
                self.tokens.expect(Token::Colon)?;
                Ok(Some(key))
            }
            other => Err(self.shape(format!("expected a key, found {}", other.describe()))),
        }
    }

    fn key_after_comma(&mut self) -> Result<Option<String>, ReadError> {
        match self.tokens.next_token()? {
            Token::String => {
                let key = self.tokens.text().to_string();
                self.tokens.expect(Token::Colon)?;
                Ok(Some(key))
            }
            other => Err(self.shape(format!("expected a key, found {}", other.describe()))),
        }
    }

    /// Next array element's first token, or `None` at the array's end.
    fn element(&mut self, first: bool) -> Result<Option<Token>, ReadError> {
        let token = self.tokens.next_token()?;
        match token {
            Token::EndArray => Ok(None),
            Token::Comma if !first => Ok(Some(self.tokens.next_token()?)),
            other if first => Ok(Some(other)),
            other => Err(self.shape(format!("expected a comma, found {}", other.describe()))),
        }
    }

    fn string_value(&mut self) -> Result<String, ReadError> {
        match self.tokens.next_token()? {
            Token::String => Ok(self.tokens.text().to_string()),
            other => Err(self.shape(format!("expected a string, found {}", other.describe()))),
        }
    }

    fn number_value<T: std::str::FromStr>(&mut self) -> Result<T, ReadError> {
        match self.tokens.next_token()? {
            Token::Number => self.parse_number(),
            other => Err(self.shape(format!("expected a number, found {}", other.describe()))),
        }
    }

    fn parse_number<T: std::str::FromStr>(&self) -> Result<T, ReadError> {
        let text = self.tokens.text();
        text.parse::<T>()
            .map_err(|_| self.shape(format!("bad number {text:?}")))
    }

    fn skip_value(&mut self, first: Token) -> Result<(), ReadError> {
        self.tokens.skip_value(first)?;
        Ok(())
    }

    fn package(&mut self) -> Result<Package, ReadError> {
        self.tokens.expect(Token::BeginObject)?;
        let mut package = Package::default();
        let mut format_ok = false;
        while let Some(key) = self.key()? {
            match key.as_str() {
                "format" => format_ok = self.string_value()? == "pages-json",
                "entries" => {
                    self.tokens.expect(Token::BeginArray)?;
                    let mut first = true;
                    while let Some(token) = self.element(first)? {
                        first = false;
                        if token != Token::BeginObject {
                            return Err(self.shape("expected an entry object"));
                        }
                        let entry = self.entry()?;
                        package.entries.push(entry);
                    }
                }
                _ => {
                    let token = self.tokens.next_token()?;
                    self.skip_value(token)?;
                }
            }
        }
        if !format_ok {
            return Err(self.shape("expected \"format\": \"pages-json\""));
        }
        match self.tokens.next_token()? {
            Token::End => Ok(package),
            other => Err(self.shape(format!(
                "unexpected {} after the document",
                other.describe()
            ))),
        }
    }

    fn entry(&mut self) -> Result<Entry, ReadError> {
        let mut name = None;
        let mut is_stream = false;
        let mut bytes = None;
        let mut stream = None;
        while let Some(key) = self.key()? {
            match key.as_str() {
                "file" => name = Some(self.string_value()?),
                "stream" => {
                    name = Some(self.string_value()?);
                    is_stream = true;
                }
                "base64" => {
                    let text = self.string_value()?;
                    let mut decoded = Vec::new();
                    base64::decode(&text, &mut decoded).ok_or_else(|| self.shape("bad base64"))?;
                    bytes = Some(decoded);
                }
                "objects" => {
                    let mut tree = Tree::new(&SCHEMA);
                    let mut objects = Vec::new();
                    self.tokens.expect(Token::BeginArray)?;
                    let mut first = true;
                    while let Some(token) = self.element(first)? {
                        first = false;
                        if token != Token::BeginObject {
                            return Err(self.shape("expected an object"));
                        }
                        objects.push(self.object(&mut tree)?);
                    }
                    stream = Some((tree, objects));
                }
                _ => {
                    let token = self.tokens.next_token()?;
                    self.skip_value(token)?;
                }
            }
        }
        let name = name.ok_or_else(|| self.shape("entry without a name"))?;
        if is_stream {
            let (tree, objects) = stream.ok_or_else(|| self.shape("stream without objects"))?;
            return Ok(Entry::Stream(Stream {
                name,
                tree,
                objects,
            }));
        }
        let bytes = bytes.ok_or_else(|| self.shape("file without base64"))?;
        Ok(Entry::File { name, bytes })
    }

    fn object(&mut self, tree: &mut Tree) -> Result<Object, ReadError> {
        let mut info = NONE;
        let mut identifier = 0u64;
        let mut messages = Vec::new();
        while let Some(key) = self.key()? {
            match key.as_str() {
                "info" => {
                    self.tokens.expect(Token::BeginObject)?;
                    info = self.message(tree, SCHEMA.message("TSP.ArchiveInfo"), None)?;
                    identifier = tree
                        .chain(info)
                        .find(|(_, entry)| entry.number == 1)
                        .and_then(|(_, entry)| match entry.value {
                            Node::Uint(identifier) => Some(identifier),
                            _ => None,
                        })
                        .unwrap_or(0);
                }
                "messages" => {
                    self.tokens.expect(Token::BeginArray)?;
                    let mut first = true;
                    while let Some(token) = self.element(first)? {
                        first = false;
                        if token != Token::BeginObject {
                            return Err(self.shape("expected a message object"));
                        }
                        messages.push(self.typed_message(tree)?);
                    }
                }
                _ => {
                    let token = self.tokens.next_token()?;
                    self.skip_value(token)?;
                }
            }
        }
        Ok(Object {
            identifier,
            info,
            messages,
        })
    }

    /// A message object with `@type` and `@id` before its fields.
    fn typed_message(&mut self, tree: &mut Tree) -> Result<ObjectMessage, ReadError> {
        let mut message_type = None;
        let mut schema = None;
        let mut chain = Chain::new();
        while let Some(key) = self.key()? {
            match key.as_str() {
                "@type" => match self.tokens.next_token()? {
                    Token::String => schema = SCHEMA.message(self.tokens.text()),
                    Token::Number => message_type = Some(self.parse_number::<u32>()?),
                    other => return Err(self.shape(format!("bad @type: {}", other.describe()))),
                },
                "@id" => {
                    let id = self.number_value::<u32>()?;
                    message_type = Some(id);
                    if schema.is_none() {
                        schema = message_schema(id);
                    }
                }
                _ => self.field(tree, &mut chain, schema, &key)?,
            }
        }
        let message_type = message_type.ok_or_else(|| self.shape("message without @id"))?;
        Ok(ObjectMessage {
            message_type,
            first: chain.first,
        })
    }

    /// The fields of a message object whose `{` has been consumed; when
    /// `first_key` is given it was already read.
    fn message(
        &mut self,
        tree: &mut Tree,
        schema: Option<MessageRef>,
        first_key: Option<String>,
    ) -> Result<u32, ReadError> {
        let mut chain = Chain::new();
        if let Some(key) = first_key {
            self.field(tree, &mut chain, schema, &key)?;
        }
        while let Some(key) = self.key()? {
            if key.starts_with('@') {
                let token = self.tokens.next_token()?;
                self.skip_value(token)?;
                continue;
            }
            self.field(tree, &mut chain, schema, &key)?;
        }
        Ok(chain.first)
    }

    fn field(
        &mut self,
        tree: &mut Tree,
        chain: &mut Chain,
        schema: Option<MessageRef>,
        key: &str,
    ) -> Result<(), ReadError> {
        let (number, known) = match key.strip_prefix('#') {
            Some(number) => (
                number
                    .parse::<u32>()
                    .map_err(|_| self.shape(format!("bad field number {key:?}")))?,
                None,
            ),
            None => {
                let message = schema.ok_or_else(|| {
                    self.shape(format!("field {key:?} in a message with no schema"))
                })?;
                let (slot, field) = message.slot_named(key).ok_or_else(|| {
                    self.shape(format!("{key:?} is not a field of {}", message.name()))
                })?;
                (field.number, Some((message, slot, field)))
            }
        };
        let token = self.tokens.next_token()?;
        let repeated = known.is_none_or(|(_, _, field)| field.repeated);
        if token == Token::BeginArray && repeated {
            let mut first = true;
            while let Some(element) = self.element(first)? {
                first = false;
                self.value(tree, chain, number, known, element)?;
            }
            return Ok(());
        }
        self.value(tree, chain, number, known, token)
    }

    fn value(
        &mut self,
        tree: &mut Tree,
        chain: &mut Chain,
        number: u32,
        known: Option<(MessageRef, u16, Field)>,
        token: Token,
    ) -> Result<(), ReadError> {
        let node = self.node(tree, known.map(|(_, _, field)| field), token)?;
        match known {
            Some((message, slot, field)) => {
                tree.push_known(chain, message, slot, field, number, node)?
            }
            None => match node {
                Node::RawVarint(_)
                | Node::RawFixed32(_)
                | Node::RawFixed64(_)
                | Node::RawBytes(_) => tree.push_unknown(chain, number, node)?,
                _ => return Err(self.shape(format!("field #{number} needs a raw_* wrapper"))),
            },
        };
        Ok(())
    }

    fn node(
        &mut self,
        tree: &mut Tree,
        field: Option<Field>,
        token: Token,
    ) -> Result<Node, ReadError> {
        let kind = field.map(|field| field.kind);
        let node = match (token, kind) {
            (Token::Number, Some(Kind::Int | Kind::Enum | Kind::Sint)) => {
                Node::Int(self.parse_number()?)
            }
            (Token::Number, Some(Kind::Uint)) => Node::Uint(self.parse_number()?),
            (Token::Number, Some(Kind::Fixed32)) => Node::Fixed32(self.parse_number()?),
            (Token::Number, Some(Kind::Fixed64)) => Node::Fixed64(self.parse_number()?),
            (Token::Number, Some(Kind::Float)) => Node::Float(self.parse_number()?),
            (Token::Number, Some(Kind::Double)) => Node::Double(self.parse_number()?),
            (Token::Number, Some(Kind::Reference)) => Node::Reference(self.parse_number()?),
            (Token::True, Some(Kind::Bool)) => Node::Bool(true),
            (Token::False, Some(Kind::Bool)) => Node::Bool(false),
            (Token::String, Some(Kind::String)) => {
                Node::Str(tree.push_bytes(self.tokens.text().as_bytes())?)
            }
            (Token::String, Some(Kind::Bytes)) => {
                let start = tree.text.len();
                base64::decode(self.tokens.text(), &mut tree.text)
                    .ok_or_else(|| self.shape("bad base64"))?;
                let length = tree.text.len() - start;
                Node::Bytes(crate::io::protobuf::tree::Span {
                    start: u32::try_from(start).map_err(|_| TreeError::TooLarge)?,
                    length: u32::try_from(length).map_err(|_| TreeError::TooLarge)?,
                })
            }
            (Token::BeginObject, kind) => return self.object_value(tree, kind),
            (other, _) => {
                return Err(self.shape(format!(
                    "{} does not fit a {} field",
                    other.describe(),
                    kind.map(|kind| format!("{kind:?}"))
                        .unwrap_or_else(|| "schemaless".to_string())
                )));
            }
        };
        Ok(node)
    }

    /// An object value: a raw wrapper, a non-finite float, or a nested message.
    fn object_value(&mut self, tree: &mut Tree, kind: Option<Kind>) -> Result<Node, ReadError> {
        let first_key = match self.key()? {
            Some(key) => key,
            None => {
                return match kind {
                    Some(Kind::Message(_)) => Ok(Node::Message(NONE)),
                    _ => Err(self.shape("empty object where a value was expected")),
                };
            }
        };
        let wrapper = match first_key.as_str() {
            "raw_varint" => Some(Node::RawVarint(self.number_value()?)),
            "raw_fixed32" => Some(Node::RawFixed32(self.number_value()?)),
            "raw_fixed64" => Some(Node::RawFixed64(self.number_value()?)),
            "float_bits" => Some(Node::Float(f32::from_bits(self.number_value()?))),
            "double_bits" => Some(Node::Double(f64::from_bits(self.number_value()?))),
            "raw_bytes" => {
                let text = self.string_value()?;
                let start = tree.text.len();
                base64::decode(&text, &mut tree.text).ok_or_else(|| self.shape("bad base64"))?;
                Some(Node::RawBytes(crate::io::protobuf::tree::Span {
                    start: u32::try_from(start).map_err(|_| TreeError::TooLarge)?,
                    length: u32::try_from(tree.text.len() - start)
                        .map_err(|_| TreeError::TooLarge)?,
                }))
            }
            _ => None,
        };
        if let Some(node) = wrapper {
            if self.key()?.is_some() {
                return Err(self.shape("a wrapper object holds exactly one member"));
            }
            return Ok(node);
        }
        match kind {
            Some(Kind::Message(index)) => {
                let nested = SCHEMA.message_at(index);
                if first_key.starts_with('@') {
                    let token = self.tokens.next_token()?;
                    self.skip_value(token)?;
                    let first = self.message(tree, nested, None)?;
                    return Ok(Node::Message(first));
                }
                let first = self.message(tree, nested, Some(first_key))?;
                Ok(Node::Message(first))
            }
            _ => Err(self.shape(format!("unexpected object member {first_key:?}"))),
        }
    }
}

pub fn read_json<R: Read>(source: R) -> Result<Package, ReadError> {
    let mut reader = Reader {
        tokens: JsonTokenizer::new(source),
    };
    reader.package()
}
