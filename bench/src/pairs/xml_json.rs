//! XML <-> JSON: generators for two shapes, our pipelines, and `quick-xml`
//! with `serde_json` as the reference pipelines, doing the same mapping
//! (attributes as `@name`, text as `#text`, repeated elements as arrays).
//!
//! Dense: a list of record elements with attributes and short child
//! elements. Prose: section elements holding paragraphs of text.

use std::fs::File;
use std::io::{BufReader, BufWriter, Write};

use quick_xml::events::{BytesEnd, BytesStart, BytesText, Event};
use sublime::converters::json_to_xml::JsonToXml;
use sublime::converters::xml_to_json::XmlToJson;

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen-xml", [units, path]) => generate_xml(units, path),
        ("gen-xml-prose", [units, path]) => generate_xml_prose(units, path),
        ("gen-json", [units, path]) => generate_json(units, path),
        ("gen-json-prose", [units, path]) => generate_json_prose(units, path),
        ("ours-xml-json", [input, output]) => run_ours(&XmlToJson, input, output),
        ("ours-json-xml", [input, output]) => run_ours(&JsonToXml, input, output),
        ("crates-xml-json", [input, output]) => crates_xml_to_json(input, output),
        ("crates-json-xml", [input, output]) => crates_json_to_xml(input, output),
        _ => Err(
            "xml-json modes: gen-xml <units> <path> | gen-xml-prose <units> <path> | gen-json <units> <path> | gen-json-prose <units> <path> | ours-xml-json <in> <out> | ours-json-xml <in> <out> | crates-xml-json <in> <out> | crates-json-xml <in> <out>"
                .to_string(),
        ),
    }
}

const WORDS: [&str; 12] = [
    "converter",
    "lossless",
    "fidelity",
    "planner",
    "arena",
    "buffer",
    "table",
    "record",
    "stream",
    "package",
    "schema",
    "document",
];

fn paragraph(index: u64, out: &mut String) {
    let sentences = 4 + (index % 5) as usize;
    for sentence in 0..sentences {
        let words = 8 + ((index + sentence as u64) % 9) as usize;
        for word in 0..words {
            let pick = WORDS[((index + sentence as u64 * 7 + word as u64 * 3) % 12) as usize];
            if word == 0 {
                let mut chars = pick.chars();
                if let Some(first) = chars.next() {
                    out.push(first.to_ascii_uppercase());
                    out.push_str(chars.as_str());
                }
            } else {
                out.push(' ');
                out.push_str(pick);
            }
        }
        out.push_str(". ");
    }
}

fn open_output(path: &str) -> Result<BufWriter<File>, String> {
    let file = File::create(path).map_err(|error| error.to_string())?;
    Ok(BufWriter::new(file))
}

fn generate_xml(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    writeln!(
        writer,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<records version=\"3\">"
    )
    .map_err(|error| error.to_string())?;
    for index in 0..count {
        let note = if index % 7 == 0 {
            "with &amp; and &lt;angles&gt;"
        } else {
            "plain"
        };
        writeln!(
            writer,
            "  <record id=\"{index}\" active=\"{}\">\n    <name>user{index}</name>\n    <email>user{index}@example.com</email>\n    <score>{}.{}</score>\n    <created>2026-09-{:02}T{:02}:00:00Z</created>\n    <tag>a</tag>\n    <tag>b{}</tag>\n    <note>{note}</note>\n    <limits min=\"{}\" max=\"{}\"/>\n  </record>",
            index % 3 == 0,
            (index * 37) % 100,
            index % 10,
            1 + index % 28,
            index % 24,
            index % 5,
            index % 50,
            50 + index % 50,
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "</records>").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn generate_xml_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    writeln!(
        writer,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<sections>"
    )
    .map_err(|error| error.to_string())?;
    let mut text = String::new();
    for index in 0..count {
        text.clear();
        paragraph(index, &mut text);
        writeln!(
            writer,
            "  <section id=\"s{index}\">\n    <heading>Section {index}</heading>\n    <body>{text}</body>\n    <words>{}</words>\n  </section>",
            text.split(' ').count()
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "</sections>").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    write!(writer, "{{\"records\":{{\"@version\":\"3\",\"record\":[")
        .map_err(|error| error.to_string())?;
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        let note = if index % 7 == 0 {
            "with & and <angles>"
        } else {
            "plain"
        };
        write!(
            writer,
            "{{\"@id\":\"{index}\",\"@active\":\"{}\",\"name\":\"user{index}\",\"email\":\"user{index}@example.com\",\"score\":\"{}.{}\",\"created\":\"2026-09-{:02}T{:02}:00:00Z\",\"tag\":[\"a\",\"b{}\"],\"note\":\"{note}\",\"limits\":{{\"@min\":\"{}\",\"@max\":\"{}\"}}}}",
            index % 3 == 0,
            (index * 37) % 100,
            index % 10,
            1 + index % 28,
            index % 24,
            index % 5,
            index % 50,
            50 + index % 50,
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "]}}}}").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn generate_json_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let mut writer = open_output(path)?;
    write!(writer, "{{\"sections\":{{\"section\":[").map_err(|error| error.to_string())?;
    let mut text = String::new();
    for index in 0..count {
        if index > 0 {
            write!(writer, ",").map_err(|error| error.to_string())?;
        }
        text.clear();
        paragraph(index, &mut text);
        write!(
            writer,
            "{{\"@id\":\"s{index}\",\"heading\":\"Section {index}\",\"body\":\"{text}\",\"words\":\"{}\"}}",
            text.split(' ').count()
        )
        .map_err(|error| error.to_string())?;
    }
    writeln!(writer, "]}}}}").map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

// ---- reference: quick-xml events into a serde_json tree, same mapping ----

struct Frame {
    name: String,
    members: serde_json::Map<String, serde_json::Value>,
    text: String,
}

fn finish_frame(frame: Frame) -> (String, serde_json::Value) {
    let text = frame.text.trim();
    let mut members = frame.members;
    if members.is_empty() {
        let value = if text.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::Value::String(text.to_string())
        };
        return (frame.name, value);
    }
    if !text.is_empty() {
        members.insert(
            "#text".to_string(),
            serde_json::Value::String(text.to_string()),
        );
    }
    (frame.name, serde_json::Value::Object(members))
}

fn add_child(
    members: &mut serde_json::Map<String, serde_json::Value>,
    name: String,
    child: serde_json::Value,
) {
    match members.get_mut(&name) {
        Some(serde_json::Value::Array(items)) => items.push(child),
        Some(existing) => {
            let first = existing.take();
            *existing = serde_json::Value::Array(vec![first, child]);
        }
        None => {
            members.insert(name, child);
        }
    }
}

fn crates_xml_to_json(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let mut reader = quick_xml::Reader::from_reader(BufReader::new(file));
    let mut buffer = Vec::new();
    let mut stack: Vec<Frame> = Vec::new();
    let mut root: Option<(String, serde_json::Value)> = None;
    loop {
        let event = reader
            .read_event_into(&mut buffer)
            .map_err(|error| error.to_string())?;
        let is_empty = matches!(event, Event::Empty(_));
        match event {
            Event::Start(start) | Event::Empty(start) => {
                let name = String::from_utf8_lossy(start.name().as_ref()).to_string();
                let mut members = serde_json::Map::new();
                for attribute in start.attributes() {
                    let attribute = attribute.map_err(|error| error.to_string())?;
                    let key = format!("@{}", String::from_utf8_lossy(attribute.key.as_ref()));
                    let value = attribute
                        .unescape_value()
                        .map_err(|error| error.to_string())?
                        .to_string();
                    members.insert(key, serde_json::Value::String(value));
                }
                let frame = Frame {
                    name,
                    members,
                    text: String::new(),
                };
                if is_empty {
                    let (name, value) = finish_frame(frame);
                    match stack.last_mut() {
                        Some(parent) => add_child(&mut parent.members, name, value),
                        None => root = Some((name, value)),
                    }
                } else {
                    stack.push(frame);
                }
            }
            Event::End(_) => {
                let Some(frame) = stack.pop() else {
                    return Err("end tag without a start".to_string());
                };
                let (name, value) = finish_frame(frame);
                match stack.last_mut() {
                    Some(parent) => add_child(&mut parent.members, name, value),
                    None => root = Some((name, value)),
                }
            }
            Event::Text(text) => {
                if let Some(frame) = stack.last_mut() {
                    let decoded = text.unescape().map_err(|error| error.to_string())?;
                    let trimmed = decoded.trim();
                    if !trimmed.is_empty() {
                        if !frame.text.is_empty() {
                            frame.text.push(' ');
                        }
                        frame.text.push_str(trimmed);
                    }
                }
            }
            Event::CData(section) => {
                if let Some(frame) = stack.last_mut() {
                    frame.text.push_str(&String::from_utf8_lossy(&section));
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buffer.clear();
    }
    let (name, value) = root.ok_or_else(|| "no root element".to_string())?;
    let mut document = serde_json::Map::new();
    document.insert(name, value);
    let mut writer = open_output(output)?;
    serde_json::to_writer(&mut writer, &serde_json::Value::Object(document))
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}

fn scalar_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}

fn write_element<W: Write>(
    writer: &mut quick_xml::Writer<W>,
    name: &str,
    value: &serde_json::Value,
) -> Result<(), String> {
    match value {
        serde_json::Value::Array(items) => {
            for item in items {
                write_element(writer, name, item)?;
            }
            Ok(())
        }
        serde_json::Value::Object(members) => {
            let mut start = BytesStart::new(name);
            let mut text: Option<String> = None;
            let mut children: Vec<(&String, &serde_json::Value)> = Vec::new();
            for (key, member) in members {
                if let Some(attribute) = key.strip_prefix('@') {
                    start.push_attribute((attribute, scalar_text(member).as_str()));
                } else if key == "#text" {
                    text = Some(scalar_text(member));
                } else {
                    children.push((key, member));
                }
            }
            if text.is_none() && children.is_empty() {
                writer
                    .write_event(Event::Empty(start))
                    .map_err(|error| error.to_string())?;
                return Ok(());
            }
            writer
                .write_event(Event::Start(start))
                .map_err(|error| error.to_string())?;
            if let Some(text) = text {
                writer
                    .write_event(Event::Text(BytesText::new(&text)))
                    .map_err(|error| error.to_string())?;
            }
            for (key, child) in children {
                write_element(writer, key, child)?;
            }
            writer
                .write_event(Event::End(BytesEnd::new(name)))
                .map_err(|error| error.to_string())
        }
        scalar => {
            let text = scalar_text(scalar);
            if text.is_empty() {
                writer
                    .write_event(Event::Empty(BytesStart::new(name)))
                    .map_err(|error| error.to_string())
            } else {
                writer
                    .write_event(Event::Start(BytesStart::new(name)))
                    .map_err(|error| error.to_string())?;
                writer
                    .write_event(Event::Text(BytesText::new(&text)))
                    .map_err(|error| error.to_string())?;
                writer
                    .write_event(Event::End(BytesEnd::new(name)))
                    .map_err(|error| error.to_string())
            }
        }
    }
}

fn crates_json_to_xml(input: &str, output: &str) -> Result<(), String> {
    let file = File::open(input).map_err(|error| error.to_string())?;
    let value: serde_json::Value =
        serde_json::from_reader(BufReader::new(file)).map_err(|error| error.to_string())?;
    let out = open_output(output)?;
    let mut writer = quick_xml::Writer::new_with_indent(out, b' ', 2);
    match &value {
        serde_json::Value::Object(members) if members.len() == 1 => {
            let (name, node) = members.iter().next().ok_or("empty")?;
            write_element(&mut writer, name, node)?;
        }
        other => write_element(&mut writer, "root", other)?,
    }
    let mut out = writer.into_inner();
    out.write_all(b"\n").map_err(|error| error.to_string())?;
    out.flush().map_err(|error| error.to_string())
}
