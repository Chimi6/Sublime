//! Development tool: dumps the object graph of an iWork package as text,
//! with every field named where the schema knows it. Built only with the
//! `dev-tools` feature, so it costs the release binary nothing.

use std::fmt::Write as _;

use crate::io::pages::package::{Entry, Object, Package, object_type_name};
use crate::io::protobuf::tree::{NONE, Node, Tree};

/// What to include in the dump.
pub struct Filter {
    /// Only this stream (an entry name such as `Index/Document.iwa`).
    pub stream: Option<String>,
    /// Only this object identifier.
    pub object: Option<u64>,
    /// Only objects of this message type.
    pub message_type: Option<u32>,
    /// Nesting depth to print; deeper messages are elided.
    pub depth: usize,
}

pub fn dump(package_bytes: &[u8], filter: &Filter, out: &mut String) -> Result<(), String> {
    let package = Package::read(package_bytes).map_err(|error| error.to_string())?;
    let mut files = Vec::new();
    for entry in &package.entries {
        match entry {
            Entry::File { name, bytes } => files.push((name.as_str(), bytes.len())),
            Entry::Stream(stream) => {
                if filter
                    .stream
                    .as_deref()
                    .is_some_and(|wanted| wanted != stream.name)
                {
                    continue;
                }
                let _ = writeln!(out, "== {} ({} objects)", stream.name, stream.objects.len());
                for object in &stream.objects {
                    if filter
                        .object
                        .is_some_and(|wanted| wanted != object.identifier)
                    {
                        continue;
                    }
                    let first_type = object.messages.first().map(|message| message.message_type);
                    if filter
                        .message_type
                        .is_some_and(|wanted| first_type != Some(wanted))
                    {
                        continue;
                    }
                    dump_object(&stream.tree, object, filter.depth, out);
                }
            }
        }
    }
    if filter.stream.is_none() && !files.is_empty() {
        let _ = writeln!(out, "== files");
        for (name, size) in files {
            let _ = writeln!(out, "  {name} ({size} bytes)");
        }
    }
    Ok(())
}

fn dump_object(tree: &Tree, object: &Object, depth: usize, out: &mut String) {
    let name = object_type_name(object);
    let _ = write!(out, "#{} {name}", object.identifier);
    if let Some(message) = object.messages.first() {
        let _ = write!(out, " ({})", message.message_type);
    }
    let _ = writeln!(out);
    let _ = writeln!(out, "  info:");
    dump_chain(tree, object.info, 2, depth, out);
    for (index, message) in object.messages.iter().enumerate() {
        let label = crate::io::pages::type_name(message.message_type).unwrap_or("?");
        let _ = writeln!(out, "  message {index}: {label} ({})", message.message_type);
        dump_chain(tree, message.first, 2, depth, out);
    }
}

fn dump_chain(tree: &Tree, first: u32, indent: usize, depth: usize, out: &mut String) {
    let pad = "  ".repeat(indent);
    for (_, entry) in tree.chain(first) {
        let key = match tree.field(entry) {
            Some(field) => field.name.to_string(),
            None => format!("#{}", entry.number),
        };
        match entry.value {
            Node::Message(nested) => {
                if indent >= depth {
                    let count = tree.chain(nested).count();
                    let _ = writeln!(out, "{pad}{key}: {{ … {count} fields }}");
                } else if nested == NONE {
                    let _ = writeln!(out, "{pad}{key}: {{}}");
                } else {
                    let _ = writeln!(out, "{pad}{key}:");
                    dump_chain(tree, nested, indent + 1, depth, out);
                }
            }
            other => {
                let _ = writeln!(out, "{pad}{key}: {}", describe(tree, other));
            }
        }
    }
}

fn describe(tree: &Tree, node: Node) -> String {
    match node {
        Node::Int(value) => value.to_string(),
        Node::Uint(value) => value.to_string(),
        Node::Bool(value) => value.to_string(),
        Node::Fixed32(value) => format!("fixed32 {value}"),
        Node::Fixed64(value) => format!("fixed64 {value}"),
        Node::Float(value) => value.to_string(),
        Node::Double(value) => value.to_string(),
        Node::Str(span) => format!("{:?}", tree.str(span)),
        Node::Bytes(span) => format!("{} bytes", span.length),
        Node::Reference(identifier) => format!("-> #{identifier}"),
        Node::Message(_) => String::new(),
        Node::RawVarint(value) => format!("raw varint {value}"),
        Node::RawFixed32(value) => format!("raw fixed32 {value}"),
        Node::RawFixed64(value) => format!("raw fixed64 {value}"),
        Node::RawBytes(span) => {
            let bytes = tree.bytes(span);
            let shown: String = bytes
                .iter()
                .take(24)
                .map(|byte| format!("{byte:02x}"))
                .collect::<Vec<_>>()
                .join(" ");
            if bytes.len() > 24 {
                format!("raw {shown} … ({} bytes)", bytes.len())
            } else {
                format!("raw {shown}")
            }
        }
    }
}
