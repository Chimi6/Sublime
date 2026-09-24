//! Development tool: dumps the object graph of an iWork package as text.
//! Built only with the `dev-tools` feature, so it costs the release binary
//! nothing. It knows no schemas; length-delimited fields are shown as
//! text when they read as text, as nested messages when they parse as one,
//! and as hex otherwise, so unknown structures can still be read.

use std::fmt::Write as _;

use crate::io::iwa::{IwaObject, decompress_stream, parse_objects};
use crate::io::pages::type_name;
use crate::io::protobuf::{FieldReader, Value};
use crate::io::zip::ZipArchive;

/// What to include in the dump.
pub struct Filter {
    /// Only this stream (an entry name such as `Index/Document.iwa`).
    pub stream: Option<String>,
    /// Only this object identifier.
    pub object: Option<u64>,
    /// Only objects of this message type.
    pub message_type: Option<u32>,
    /// Nesting depth to decode; deeper messages are shown as hex.
    pub depth: usize,
}

pub fn dump(package: &[u8], filter: &Filter, out: &mut String) -> Result<(), String> {
    let archive = ZipArchive::parse(package).map_err(|error| error.to_string())?;
    let mut compressed = Vec::new();
    for entry in archive.entries() {
        if !entry.name.ends_with(".iwa") {
            continue;
        }
        if filter
            .stream
            .as_deref()
            .is_some_and(|wanted| wanted != entry.name)
        {
            continue;
        }
        compressed.clear();
        archive
            .read(entry, &mut compressed)
            .map_err(|error| error.to_string())?;
        let stream =
            decompress_stream(&compressed).map_err(|error| format!("{}: {error}", entry.name))?;
        let objects = parse_objects(&stream).map_err(|error| format!("{}: {error}", entry.name))?;
        let _ = writeln!(
            out,
            "== {} ({} objects, {} bytes)",
            entry.name,
            objects.len(),
            stream.len()
        );
        for object in &objects {
            if filter
                .object
                .is_some_and(|wanted| wanted != object.identifier)
            {
                continue;
            }
            if filter
                .message_type
                .is_some_and(|wanted| object.message_type() != Some(wanted))
            {
                continue;
            }
            dump_object(object, filter.depth, out);
        }
    }
    let mut others: Vec<&str> = archive
        .entries()
        .iter()
        .filter(|entry| !entry.name.ends_with(".iwa") && !entry.is_directory())
        .map(|entry| entry.name.as_str())
        .collect();
    others.sort_unstable();
    if filter.stream.is_none() && !others.is_empty() {
        let _ = writeln!(out, "== other entries");
        for name in others {
            let _ = writeln!(out, "  {name}");
        }
    }
    Ok(())
}

fn dump_object(object: &IwaObject<'_>, depth: usize, out: &mut String) {
    for (index, message) in object.messages.iter().enumerate() {
        let name = type_name(message.message_type).unwrap_or("?");
        if index == 0 {
            let _ = write!(
                out,
                "#{} {name} ({})",
                object.identifier, message.message_type
            );
        } else {
            let _ = write!(
                out,
                "#{} +{name} ({})",
                object.identifier, message.message_type
            );
        }
        if !message.object_references.is_empty() {
            let _ = write!(out, " refs={:?}", message.object_references);
        }
        if !message.data_references.is_empty() {
            let _ = write!(out, " data={:?}", message.data_references);
        }
        let _ = writeln!(out, " [{} bytes]", message.payload.len());
        dump_message(message.payload, 1, depth, out);
    }
}

fn dump_message(bytes: &[u8], indent: usize, depth: usize, out: &mut String) {
    for field in FieldReader::new(bytes) {
        let field = match field {
            Ok(field) => field,
            Err(error) => {
                let _ = writeln!(out, "{}!! {error}", "  ".repeat(indent));
                return;
            }
        };
        let pad = "  ".repeat(indent);
        match field.value {
            Value::Varint(value) => {
                let _ = writeln!(out, "{pad}{}: {value}", field.number);
            }
            Value::Fixed64(value) => {
                let _ = writeln!(
                    out,
                    "{pad}{}: fixed64 {value} (f64 {})",
                    field.number,
                    f64::from_bits(value)
                );
            }
            Value::Fixed32(value) => {
                let _ = writeln!(
                    out,
                    "{pad}{}: fixed32 {value} (f32 {})",
                    field.number,
                    f32::from_bits(value)
                );
            }
            Value::Group(group) => {
                let _ = writeln!(out, "{pad}{}: group", field.number);
                dump_message(group, indent + 1, depth, out);
            }
            Value::Bytes(bytes) => dump_bytes(field.number, bytes, indent, depth, out),
        }
    }
}

fn dump_bytes(number: u32, bytes: &[u8], indent: usize, depth: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    if bytes.is_empty() {
        let _ = writeln!(out, "{pad}{number}: \"\"");
        return;
    }
    if let Some(text) = readable_text(bytes) {
        let _ = writeln!(out, "{pad}{number}: {text:?}");
        return;
    }
    if indent < depth && parses_as_message(bytes) {
        let _ = writeln!(out, "{pad}{number}: {{");
        dump_message(bytes, indent + 1, depth, out);
        let _ = writeln!(out, "{pad}}}");
        return;
    }
    let shown: String = bytes
        .iter()
        .take(48)
        .map(|byte| format!("{byte:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    let more = if bytes.len() > 48 {
        format!(" … ({} bytes)", bytes.len())
    } else {
        String::new()
    };
    let _ = writeln!(out, "{pad}{number}: hex {shown}{more}");
}

/// Text if every character is printable or ordinary whitespace.
fn readable_text(bytes: &[u8]) -> Option<&str> {
    let text = std::str::from_utf8(bytes).ok()?;
    let printable = text
        .chars()
        .all(|ch| !ch.is_control() || matches!(ch, '\n' | '\t' | '\r'));
    if printable { Some(text) } else { None }
}

/// True when the bytes decode as a whole message with plausible fields.
fn parses_as_message(bytes: &[u8]) -> bool {
    let mut count = 0usize;
    for field in FieldReader::new(bytes) {
        match field {
            Ok(field) if field.number > 0 && field.number < 100_000 => count += 1,
            _ => return false,
        }
    }
    count > 0
}
