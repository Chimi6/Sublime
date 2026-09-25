//! TOML <-> JSON oracles over the fixtures in `tests/fixtures/toml`: every
//! valid document reads to the JSON beside it, comes back through the TOML
//! writer to the same JSON, every invalid document is refused, and the
//! JSON-first fixtures write the TOML beside them.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::json_to_toml::JsonToToml;
use sublime::converters::toml_to_json::TomlToJson;
use sublime::event::{CollectingSink, Context, Event};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/toml")
        .join(sub)
}

fn convert(converter: &dyn Converter, bytes: &[u8]) -> (Result<Vec<u8>, ConvertError>, Vec<Event>) {
    let options = ConvertOptions::default();
    let mut sink = CollectingSink::new();
    let mut output = Vec::new();
    let result = {
        let mut context = Context::new(&mut sink, &options);
        let mut cursor = std::io::Cursor::new(bytes);
        converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)
    };
    let events = sink.into_events();
    match result {
        Ok(()) => (Ok(output), events),
        Err(error) => (Err(error), events),
    }
}

fn losses(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .filter_map(|event| match event {
            Event::LossDetected { description, .. } => Some(description.clone()),
            _ => None,
        })
        .collect()
}

fn toml_fixtures(sub: &str, extension: &str) -> Vec<(String, Vec<u8>)> {
    let mut names: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    names.sort();
    assert!(!names.is_empty(), "no {extension} fixtures in {sub:?}");
    names
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            let bytes = fs::read(&path).expect("read fixture");
            (name, bytes)
        })
        .collect()
}

#[test]
fn every_valid_toml_reads_to_the_json_beside_it() {
    for (name, toml) in toml_fixtures("", "toml") {
        let expected = fs::read(fixture_dir("").join(format!("{name}.json"))).expect("json");
        let (result, _) = convert(&TomlToJson, &toml);
        let json = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            String::from_utf8_lossy(&json),
            String::from_utf8_lossy(&expected),
            "{name}"
        );
    }
}

#[test]
fn every_valid_toml_survives_the_round_trip_through_the_writer() {
    for (name, toml) in toml_fixtures("", "toml") {
        let (first, _) = convert(&TomlToJson, &toml);
        let json = first.expect("toml -> json");
        let (back, _) = convert(&JsonToToml, &json);
        let toml_again = back.unwrap_or_else(|error| panic!("{name}: {error}"));
        let (second, _) = convert(&TomlToJson, &toml_again);
        let json_again = second.unwrap_or_else(|error| {
            panic!(
                "{name}: our TOML does not read back: {error}\n{}",
                String::from_utf8_lossy(&toml_again)
            )
        });
        // TOML lays a table out as plain members then sub-tables, so the
        // trees are compared with members sorted, not the bytes.
        assert_eq!(sorted_tree(&json_again), sorted_tree(&json), "{name}");
    }
}

/// The JSON re-parsed and written back with every table's members
/// sorted by key, so two documents that differ only in member order
/// compare equal.
fn sorted_tree(json: &[u8]) -> String {
    let mut tree = sublime::io::json::parse(json).expect("our JSON parses");
    let root = tree.root;
    sort_members(&mut tree, root);
    sublime::io::json::from_tree::compact_text(&tree, tree.root)
}

fn sort_members(tree: &mut sublime::value::Tree, node: u32) {
    use sublime::value::{Children, Data, NONE};
    let children: Vec<u32> = tree.children(node).collect();
    for child in &children {
        sort_members(tree, *child);
    }
    if let Data::Table(_) = tree.data(node) {
        let mut sorted = children;
        sorted.sort_by(|left, right| tree.key(*left).cmp(tree.key(*right)));
        for pair in sorted.windows(2) {
            tree.nodes[pair[0] as usize].next = pair[1];
        }
        if let Some(last) = sorted.last() {
            tree.nodes[*last as usize].next = NONE;
        }
        tree.nodes[node as usize].data = Data::Table(Children {
            first: sorted.first().copied().unwrap_or(NONE),
            last: sorted.last().copied().unwrap_or(NONE),
        });
    }
}

#[test]
fn every_invalid_toml_is_refused_as_malformed() {
    for (name, toml) in toml_fixtures("invalid", "toml") {
        let (result, _) = convert(&TomlToJson, &toml);
        match result {
            Err(ConvertError::Malformed { .. }) => {}
            Err(other) => panic!("{name}: expected Malformed, got {other}"),
            Ok(json) => panic!("{name}: accepted: {}", String::from_utf8_lossy(&json)),
        }
    }
}

#[test]
fn json_writes_the_toml_beside_it_and_reports_dropped_nulls() {
    let json = fs::read(fixture_dir("from-json").join("kitchen.json")).expect("json");
    let expected = fs::read(fixture_dir("from-json").join("kitchen.toml")).expect("toml");
    let (result, events) = convert(&JsonToToml, &json);
    let toml = result.expect("json -> toml");
    assert_eq!(
        String::from_utf8_lossy(&toml),
        String::from_utf8_lossy(&expected)
    );
    let dropped = losses(&events);
    assert_eq!(dropped.len(), 2, "{dropped:?}");
    assert!(dropped[0].contains("none"), "{dropped:?}");
    assert!(dropped[1].contains("list"), "{dropped:?}");
}

#[test]
fn json_roots_that_are_not_objects_are_unsupported() {
    for name in ["array_root.json", "scalar_root.json"] {
        let json = fs::read(fixture_dir("from-json").join(name)).expect("json");
        let (result, _) = convert(&JsonToToml, &json);
        match result {
            Err(ConvertError::Unsupported(message)) => {
                assert!(message.contains("table"), "{name}: {message}")
            }
            Err(other) => panic!("{name}: expected Unsupported, got {other}"),
            Ok(_) => panic!("{name}: accepted"),
        }
    }
}

#[test]
fn toml_to_json_reports_what_became_strings() {
    let toml = b"when = 1979-05-27\nhuge = inf\nmissing = nan\n[[row]]\nat = 07:32:00\n[[row]]\nat = 07:33:00\n";
    let (result, events) = convert(&TomlToJson, toml);
    assert_eq!(
        String::from_utf8_lossy(&result.expect("toml -> json")),
        r#"{"when":"1979-05-27","huge":"inf","missing":"nan","row":[{"at":"07:32:00"},{"at":"07:33:00"}]}"#
    );
    let noted = losses(&events);
    assert_eq!(noted.len(), 4, "{noted:?}");
    assert!(noted[3].starts_with("row[].at:"), "{noted:?}");
}
