//! Pages <-> `pages-json`, and the generator every Pages pair shares. A
//! large document is made by scaling a fixture: its body text is repeated
//! `units` times and every character-indexed attribute table is repeated
//! with it, through the lossless JSON form and back into a package. The
//! result is a real Pages package whose text, styles, and lists grow with
//! `units` while the stylesheet and the rest stay as Pages wrote them.

use std::fs::File;
use std::io::BufWriter;

use serde_json::Value;
use sublime::converters::json_to_pages::JsonToPages;
use sublime::converters::pages_to_json::PagesToJson;
use sublime::io::pages::Package;
use sublime::io::pages::json::{read_json, write_json};

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen", [seed, units, path]) => generate(seed, units, path),
        ("ours", [input, output]) => run_ours(&PagesToJson, input, output),
        ("ours-back", [input, output]) => run_ours(&JsonToPages, input, output),
        _ => Err(
            "pages-json modes: gen <seed.pages> <units> <out.pages> | ours <in> <out> | ours-back <in> <out>"
                .to_string(),
        ),
    }
}

/// Writes `seed` with its body text and attribute tables repeated `units`
/// times to `path`.
pub fn generate(seed: &str, units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?.max(1) as usize;
    let bytes = std::fs::read(seed).map_err(|error| format!("{seed}: {error}"))?;
    let package = Package::read(&bytes).map_err(|error| error.to_string())?;
    let mut json = Vec::new();
    write_json(&package, &mut json).map_err(|error| error.to_string())?;
    let mut document: Value = serde_json::from_slice(&json).map_err(|error| error.to_string())?;
    let storage = body_storage(&mut document).ok_or("no body storage in the seed")?;
    scale_storage(storage, count)?;
    let scaled = serde_json::to_vec(&document).map_err(|error| error.to_string())?;
    let package = read_json(scaled.as_slice()).map_err(|error| error.to_string())?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    package
        .write(BufWriter::new(file))
        .map_err(|error| error.to_string())?;
    Ok(())
}

/// The first in-document body storage message of `Index/Document.iwa`.
fn body_storage(document: &mut Value) -> Option<&mut Value> {
    let entries = document.get_mut("entries")?.as_array_mut()?;
    let stream = entries
        .iter_mut()
        .find(|entry| entry.get("stream").and_then(Value::as_str) == Some("Index/Document.iwa"))?;
    let objects = stream.get_mut("objects")?.as_array_mut()?;
    for object in objects {
        let messages = object.get_mut("messages")?.as_array_mut()?;
        let Some(message) = messages.first_mut() else {
            continue;
        };
        let is_body = message.get("@type").and_then(Value::as_str) == Some("TSWP.StorageArchive")
            && message.get("kind").and_then(Value::as_i64).unwrap_or(0) == 0
            && message.get("in_document").and_then(Value::as_bool) == Some(true)
            && message.get("text").is_some();
        if is_body {
            return Some(message);
        }
    }
    None
}

/// Repeats the text `count` times with a paragraph break between copies
/// and shifts every attribute table's entries to match.
fn scale_storage(storage: &mut Value, count: usize) -> Result<(), String> {
    let text: String = match storage.get("text") {
        Some(Value::Array(parts)) => parts.iter().filter_map(Value::as_str).collect(),
        Some(Value::String(text)) => text.clone(),
        _ => return Err("body storage has no text".to_string()),
    };
    let unit_length = text.encode_utf16().count();
    let stride = unit_length + 1;
    let mut scaled = String::with_capacity((text.len() + 1) * count);
    for copy in 0..count {
        if copy > 0 {
            scaled.push('\n');
        }
        scaled.push_str(&text);
    }
    storage["text"] = Value::Array(vec![Value::String(scaled)]);
    let Some(fields) = storage.as_object_mut() else {
        return Err("body storage is not an object".to_string());
    };
    for (name, table) in fields.iter_mut() {
        if !name.starts_with("table_") {
            continue;
        }
        let Some(entries) = table.get_mut("entries").and_then(Value::as_array_mut) else {
            continue;
        };
        let original = entries.clone();
        entries.clear();
        for copy in 0..count {
            for entry in &original {
                let mut shifted = entry.clone();
                let index = entry
                    .get("character_index")
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                // An entry at the very end of the text closes the last
                // copy only; in between it would shadow the next copy's
                // first entry.
                if index as usize >= unit_length && copy + 1 < count {
                    continue;
                }
                shifted["character_index"] = Value::from(index + (copy * stride) as u64);
                entries.push(shifted);
            }
        }
    }
    Ok(())
}
