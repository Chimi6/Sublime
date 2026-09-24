//! A Pages package as JSON (`pages-json`): every entry, with each object's
//! header and messages as schema-named trees. Lossless at the object level:
//! reading the JSON back gives trees that re-encode to the original bytes.
//!
//! ```text
//! {"format":"pages-json","version":1,"entries":[
//!   {"stream":"Index/Document.iwa","objects":[
//!     {"info":{"@type":"TSP.ArchiveInfo","identifier":1,"message_infos":[{...}]},
//!      "messages":[{"@type":"TP.DocumentArchive","stylesheet":1686817,...}]}]},
//!   {"file":"Data/image1.png","base64":"..."}]}
//! ```
//!
//! Field values: numbers, booleans, and strings as expected; references as
//! the object identifier; bytes as base64; repeated fields as arrays; a
//! float that is not finite as `{"float_bits":n}` or `{"double_bits":n}`.
//! Fields without a schema are keyed `"#<number>"` and wrapped by wire
//! type: `{"raw_varint":n}`, `{"raw_fixed32":n}`, `{"raw_fixed64":n}`,
//! `{"raw_bytes":"base64"}`; a known field whose value could not be
//! decoded as its kind carries the same wrappers.

use std::io::{self, Read, Write};

use super::package::{Entry, Object, ObjectMessage, Package};
use super::schema::SCHEMA;
use super::type_name;
use crate::io::base64;
use crate::io::json::{JsonError, JsonValue, JsonWriter};
use crate::io::protobuf::schema::{Field, Kind, MessageRef};
use crate::io::protobuf::tree::{Entry as TreeEntry, Fields, Node};

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
    let mut scratch = String::new();
    for entry in &package.entries {
        json.begin_object()?;
        match entry {
            Entry::File { name, bytes } => {
                json.key("file")?;
                json.string(name)?;
                json.key("base64")?;
                scratch.clear();
                base64::encode(bytes, &mut scratch);
                json.string(&scratch)?;
            }
            Entry::Stream { name, objects } => {
                json.key("stream")?;
                json.string(name)?;
                json.key("objects")?;
                json.begin_array()?;
                for object in objects {
                    write_object(&mut json, object, &mut scratch)?;
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
    object: &Object,
    scratch: &mut String,
) -> io::Result<()> {
    json.begin_object()?;
    json.key("info")?;
    write_message(json, &object.info, Some("TSP.ArchiveInfo"), scratch)?;
    json.key("messages")?;
    json.begin_array()?;
    for message in &object.messages {
        let name = type_name(message.message_type);
        write_message_typed(json, &message.fields, message.message_type, name, scratch)?;
    }
    json.end_array()?;
    json.end_object()
}

fn write_message_typed<W: Write>(
    json: &mut JsonWriter<W>,
    fields: &Fields,
    message_type: u32,
    name: Option<&str>,
    scratch: &mut String,
) -> io::Result<()> {
    json.begin_object()?;
    json.key("@type")?;
    match name {
        Some(name) => json.string(name)?,
        None => json.raw(&message_type.to_string())?,
    }
    if name.is_some() {
        json.key("@id")?;
        json.raw(&message_type.to_string())?;
    }
    write_fields(json, fields, scratch)?;
    json.end_object()
}

fn write_message<W: Write>(
    json: &mut JsonWriter<W>,
    fields: &Fields,
    name: Option<&str>,
    scratch: &mut String,
) -> io::Result<()> {
    json.begin_object()?;
    if let Some(name) = name {
        json.key("@type")?;
        json.string(name)?;
    }
    write_fields(json, fields, scratch)?;
    json.end_object()
}

/// Consecutive entries of one field become an array when the field is
/// repeated (or unknown and repeated in the data).
fn write_fields<W: Write>(
    json: &mut JsonWriter<W>,
    fields: &Fields,
    scratch: &mut String,
) -> io::Result<()> {
    let entries = &fields.entries;
    let mut index = 0usize;
    while index < entries.len() {
        let entry = &entries[index];
        let mut run_end = index + 1;
        while run_end < entries.len()
            && entries[run_end].number == entry.number
            && entries[run_end].field == entry.field
        {
            run_end += 1;
        }
        let repeated = entry.field.is_some_and(|field| field.repeated) || run_end - index > 1;
        match entry.field {
            Some(field) => json.key(field.name)?,
            None => json.key(&format!("#{}", entry.number))?,
        }
        if repeated {
            json.begin_array()?;
            for element in &entries[index..run_end] {
                write_node(json, &element.value, element.field, scratch)?;
            }
            json.end_array()?;
        } else {
            write_node(json, &entry.value, entry.field, scratch)?;
        }
        index = run_end;
    }
    Ok(())
}

fn write_node<W: Write>(
    json: &mut JsonWriter<W>,
    node: &Node,
    field: Option<Field>,
    scratch: &mut String,
) -> io::Result<()> {
    match node {
        Node::Int(value) => json.raw(&value.to_string()),
        Node::Uint(value) => json.raw(&value.to_string()),
        Node::Bool(value) => json.raw(if *value { "true" } else { "false" }),
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
        Node::Str(text) => json.string(text),
        Node::Bytes(bytes) => {
            scratch.clear();
            base64::encode(bytes, scratch);
            json.string(scratch)
        }
        Node::Reference(identifier) => json.raw(&identifier.to_string()),
        Node::Message(fields) => {
            let name = match field.map(|field| field.kind) {
                Some(Kind::Message(index)) => {
                    SCHEMA.message_at(index).map(|message| message.name())
                }
                _ => None,
            };
            write_message(json, fields, name, scratch)
        }
        Node::RawVarint(value) => wrapped(json, "raw_varint", &value.to_string()),
        Node::RawFixed32(value) => wrapped(json, "raw_fixed32", &value.to_string()),
        Node::RawFixed64(value) => wrapped(json, "raw_fixed64", &value.to_string()),
        Node::RawBytes(bytes) => {
            scratch.clear();
            base64::encode(bytes, scratch);
            json.begin_object()?;
            json.key("raw_bytes")?;
            json.string(scratch)?;
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
    /// A structural problem, with the path that has it.
    Shape {
        path: String,
        what: String,
    },
}

impl std::fmt::Display for ReadError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReadError::Json(error) => write!(formatter, "{error}"),
            ReadError::Shape { path, what } => write!(formatter, "{path}: {what}"),
        }
    }
}

impl std::error::Error for ReadError {}

impl From<JsonError> for ReadError {
    fn from(error: JsonError) -> Self {
        ReadError::Json(error)
    }
}

fn shape(path: &str, what: impl Into<String>) -> ReadError {
    ReadError::Shape {
        path: path.to_string(),
        what: what.into(),
    }
}

pub fn read_json<R: Read>(source: R) -> Result<Package, ReadError> {
    let document = crate::io::json::value::parse(source)?;
    if document.get("format").and_then(JsonValue::as_str) != Some("pages-json") {
        return Err(shape("format", "expected \"pages-json\""));
    }
    let entries = document
        .get("entries")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| shape("entries", "expected an array"))?;
    let mut package = Package::default();
    let mut bytes = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let path = format!("entries[{index}]");
        if let Some(name) = entry.get("file").and_then(JsonValue::as_str) {
            let text = entry
                .get("base64")
                .and_then(JsonValue::as_str)
                .ok_or_else(|| shape(&path, "file without base64"))?;
            bytes.clear();
            base64::decode(text, &mut bytes).ok_or_else(|| shape(&path, "bad base64"))?;
            package.entries.push(Entry::File {
                name: name.to_string(),
                bytes: bytes.clone(),
            });
            continue;
        }
        let name = entry
            .get("stream")
            .and_then(JsonValue::as_str)
            .ok_or_else(|| shape(&path, "entry is neither a file nor a stream"))?;
        let objects = entry
            .get("objects")
            .and_then(JsonValue::as_array)
            .ok_or_else(|| shape(&path, "stream without objects"))?;
        let mut decoded = Vec::with_capacity(objects.len());
        for (object_index, object) in objects.iter().enumerate() {
            let object_path = format!("{path}.objects[{object_index}]");
            decoded.push(read_object(object, &object_path)?);
        }
        package.entries.push(Entry::Stream {
            name: name.to_string(),
            objects: decoded,
        });
    }
    Ok(package)
}

fn read_object(value: &JsonValue, path: &str) -> Result<Object, ReadError> {
    let info_value = value
        .get("info")
        .ok_or_else(|| shape(path, "object without info"))?;
    let info_schema = SCHEMA.message("TSP.ArchiveInfo");
    let info = read_message(info_value, info_schema, &format!("{path}.info"))?;
    let identifier = info
        .entries
        .iter()
        .find(|entry| entry.number == 1)
        .and_then(|entry| match entry.value {
            Node::Uint(identifier) => Some(identifier),
            _ => None,
        })
        .unwrap_or(0);
    let messages_value = value
        .get("messages")
        .and_then(JsonValue::as_array)
        .ok_or_else(|| shape(path, "object without messages"))?;
    let mut messages = Vec::with_capacity(messages_value.len());
    for (index, message) in messages_value.iter().enumerate() {
        let message_path = format!("{path}.messages[{index}]");
        let message_type = match (message.get("@id"), message.get("@type")) {
            (Some(JsonValue::Number(text)), _) | (None, Some(JsonValue::Number(text))) => text
                .parse::<u32>()
                .map_err(|_| shape(&message_path, "bad @type number"))?,
            _ => {
                return Err(shape(
                    &message_path,
                    "message without a numeric @id or @type",
                ));
            }
        };
        let schema = message
            .get("@type")
            .and_then(JsonValue::as_str)
            .and_then(|name| SCHEMA.message(name));
        let fields = read_message(message, schema, &message_path)?;
        messages.push(ObjectMessage {
            message_type,
            fields,
        });
    }
    Ok(Object {
        identifier,
        info,
        messages,
    })
}

fn read_message(
    value: &JsonValue,
    schema: Option<MessageRef>,
    path: &str,
) -> Result<Fields, ReadError> {
    let members = match value {
        JsonValue::Object(members) => members,
        _ => return Err(shape(path, "expected an object")),
    };
    let mut fields = Fields::default();
    for (key, member) in members {
        if key.starts_with('@') {
            continue;
        }
        let member_path = format!("{path}.{key}");
        let (number, field) = match key.strip_prefix('#') {
            Some(number) => (
                number
                    .parse::<u32>()
                    .map_err(|_| shape(&member_path, "bad field number"))?,
                None,
            ),
            None => {
                let field = schema
                    .and_then(|message| message.field_named(key))
                    .ok_or_else(|| shape(&member_path, "unknown field name for this message"))?;
                (field.number, Some(field))
            }
        };
        let repeated = field.is_some_and(|field| field.repeated);
        match member {
            JsonValue::Array(items) if repeated || field.is_none() => {
                for item in items {
                    let node = read_node(item, field, &member_path)?;
                    fields.entries.push(TreeEntry {
                        number,
                        field,
                        value: node,
                    });
                }
            }
            other => {
                let node = read_node(other, field, &member_path)?;
                fields.entries.push(TreeEntry {
                    number,
                    field,
                    value: node,
                });
            }
        }
    }
    Ok(fields)
}

fn read_node(value: &JsonValue, field: Option<Field>, path: &str) -> Result<Node, ReadError> {
    if let JsonValue::Object(members) = value
        && members.len() == 1
    {
        let (key, inner) = &members[0];
        let raw = match (key.as_str(), inner) {
            ("raw_varint", JsonValue::Number(text)) => {
                Some(Node::RawVarint(parse_number(text, path)?))
            }
            ("raw_fixed32", JsonValue::Number(text)) => {
                Some(Node::RawFixed32(parse_number(text, path)?))
            }
            ("raw_fixed64", JsonValue::Number(text)) => {
                Some(Node::RawFixed64(parse_number(text, path)?))
            }
            ("raw_bytes", JsonValue::String(text)) => {
                let mut bytes = Vec::new();
                base64::decode(text, &mut bytes).ok_or_else(|| shape(path, "bad base64"))?;
                Some(Node::RawBytes(bytes))
            }
            ("float_bits", JsonValue::Number(text)) => {
                Some(Node::Float(f32::from_bits(parse_number(text, path)?)))
            }
            ("double_bits", JsonValue::Number(text)) => {
                Some(Node::Double(f64::from_bits(parse_number(text, path)?)))
            }
            _ => None,
        };
        if let Some(raw) = raw {
            return Ok(raw);
        }
    }
    let kind = match field {
        Some(field) => field.kind,
        None => return Err(shape(path, "unknown field needs a raw_* wrapper")),
    };
    let node = match (kind, value) {
        (Kind::Int | Kind::Enum | Kind::Sint, JsonValue::Number(text)) => {
            Node::Int(parse_number(text, path)?)
        }
        (Kind::Uint, JsonValue::Number(text)) => Node::Uint(parse_number(text, path)?),
        (Kind::Bool, JsonValue::Bool(value)) => Node::Bool(*value),
        (Kind::Fixed32, JsonValue::Number(text)) => Node::Fixed32(parse_number(text, path)?),
        (Kind::Fixed64, JsonValue::Number(text)) => Node::Fixed64(parse_number(text, path)?),
        (Kind::Float, JsonValue::Number(text)) => Node::Float(parse_number(text, path)?),
        (Kind::Double, JsonValue::Number(text)) => Node::Double(parse_number(text, path)?),
        (Kind::String, JsonValue::String(text)) => Node::Str(text.clone()),
        (Kind::Bytes, JsonValue::String(text)) => {
            let mut bytes = Vec::new();
            base64::decode(text, &mut bytes).ok_or_else(|| shape(path, "bad base64"))?;
            Node::Bytes(bytes)
        }
        (Kind::Reference, JsonValue::Number(text)) => Node::Reference(parse_number(text, path)?),
        (Kind::Message(index), JsonValue::Object(_)) => {
            let schema = SCHEMA.message_at(index);
            Node::Message(read_message(value, schema, path)?)
        }
        _ => return Err(shape(path, "value does not match the field's kind")),
    };
    Ok(node)
}

fn parse_number<T: std::str::FromStr>(text: &str, path: &str) -> Result<T, ReadError> {
    text.parse::<T>()
        .map_err(|_| shape(path, format!("bad number {text:?}")))
}
