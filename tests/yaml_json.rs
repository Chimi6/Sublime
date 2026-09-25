//! YAML <-> JSON oracles over `tests/fixtures/yaml`: every valid document
//! reads to the JSON beside it, every invalid document is refused, the
//! JSON-first fixtures write the YAML beside them, and JSON survives the
//! trip through YAML and back as the same tree (JSON is a subset of YAML).

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::json_to_yaml::JsonToYaml;
use sublime::converters::yaml_to_json::YamlToJson;
use sublime::event::{CollectingSink, Context, Event};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/yaml")
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

fn fixtures(sub: &str, extension: &str) -> Vec<(String, Vec<u8>)> {
    let mut paths: Vec<PathBuf> = fs::read_dir(fixture_dir(sub))
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == extension))
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no {extension} fixtures in {sub:?}");
    paths
        .into_iter()
        .map(|path| {
            let name = path
                .file_stem()
                .expect("stem")
                .to_string_lossy()
                .to_string();
            (name, fs::read(&path).expect("read fixture"))
        })
        .collect()
}

#[test]
fn every_valid_yaml_reads_to_the_json_beside_it() {
    for (name, yaml) in fixtures("", "yaml") {
        let expected = fs::read(fixture_dir("").join(format!("{name}.json"))).expect("json");
        let (result, _) = convert(&YamlToJson, &yaml);
        let json = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            String::from_utf8_lossy(&json),
            String::from_utf8_lossy(&expected),
            "{name}"
        );
    }
}

#[test]
fn every_invalid_yaml_is_refused_as_malformed() {
    for (name, yaml) in fixtures("invalid", "yaml") {
        let (result, _) = convert(&YamlToJson, &yaml);
        match result {
            Err(ConvertError::Malformed { .. }) => {}
            Err(other) => panic!("{name}: expected Malformed, got {other}"),
            Ok(json) => panic!("{name}: accepted: {}", String::from_utf8_lossy(&json)),
        }
    }
}

#[test]
fn json_writes_the_yaml_beside_it() {
    for (name, json) in fixtures("from-json", "json") {
        let expected =
            fs::read(fixture_dir("from-json").join(format!("{name}.yaml"))).expect("yaml");
        let (result, events) = convert(&JsonToYaml, &json);
        let yaml = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            String::from_utf8_lossy(&yaml),
            String::from_utf8_lossy(&expected),
            "{name}"
        );
        assert!(losses(&events).is_empty(), "{name}: {:?}", losses(&events));
    }
}

#[test]
fn json_survives_yaml_and_back_byte_for_byte() {
    let mut inputs = fixtures("", "json");
    inputs.extend(fixtures("from-json", "json"));
    inputs.extend(
        fs::read_dir(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/toml"))
            .expect("toml fixtures")
            .map(|entry| entry.expect("entry").path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
            .map(|path| {
                let name = format!("toml/{}", path.file_stem().expect("stem").to_string_lossy());
                (name, fs::read(&path).expect("read"))
            }),
    );
    for (name, json) in inputs {
        let (to_yaml, _) = convert(&JsonToYaml, &json);
        let yaml = to_yaml.unwrap_or_else(|error| panic!("{name}: json -> yaml: {error}"));
        let (back, events) = convert(&YamlToJson, &yaml);
        let json_again = back.unwrap_or_else(|error| {
            panic!(
                "{name}: our YAML does not read back: {error}\n{}",
                String::from_utf8_lossy(&yaml)
            )
        });
        // The original JSON may carry whitespace; the trees must match.
        let original = sublime::io::json::parse(&json[..]).expect("fixture JSON parses");
        let returned = sublime::io::json::parse(&json_again[..]).expect("our JSON parses");
        assert_eq!(
            sublime::io::json::from_tree::compact_text(&returned, returned.root),
            sublime::io::json::from_tree::compact_text(&original, original.root),
            "{name}\n{}",
            String::from_utf8_lossy(&yaml)
        );
        assert!(losses(&events).is_empty(), "{name}: {:?}", losses(&events));
    }
}

#[test]
fn yaml_to_json_reports_what_it_could_not_keep() {
    let yaml = b"a: !custom thing\nb: .inf\n? [1, 2]\n: seq key\nc: !custom other\n";
    let (result, events) = convert(&YamlToJson, yaml);
    assert_eq!(
        String::from_utf8_lossy(&result.expect("yaml -> json")),
        r#"{"a":"thing","b":"inf","[1,2]":"seq key","c":"other"}"#
    );
    let noted = losses(&events);
    assert_eq!(noted.len(), 3, "{noted:?}");
    assert!(noted[0].contains("!custom"), "{noted:?}");
    assert!(noted[1].contains("key"), "{noted:?}");
    assert!(noted[2].starts_with("b:"), "{noted:?}");
}
