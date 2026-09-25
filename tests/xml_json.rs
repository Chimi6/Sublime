//! XML <-> JSON oracles over `tests/fixtures/xml`: every well-formed
//! document reads to the JSON beside it under the documented mapping,
//! survives XML to JSON to XML to JSON as the same tree, every malformed
//! document is refused, and the JSON-first fixtures write the XML beside
//! them or are refused where the mapping has no answer.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::json_to_xml::JsonToXml;
use sublime::converters::xml_to_json::XmlToJson;
use sublime::event::{CollectingSink, Context, Event};

fn fixture_dir(sub: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/xml")
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
fn every_well_formed_xml_reads_to_the_json_beside_it() {
    for (name, xml) in fixtures("", "xml") {
        let expected = fs::read(fixture_dir("").join(format!("{name}.json"))).expect("json");
        let (result, _) = convert(&XmlToJson, &xml);
        let json = result.unwrap_or_else(|error| panic!("{name}: {error}"));
        assert_eq!(
            String::from_utf8_lossy(&json),
            String::from_utf8_lossy(&expected),
            "{name}"
        );
    }
}

#[test]
fn every_well_formed_xml_survives_the_round_trip() {
    for (name, xml) in fixtures("", "xml") {
        let (first, _) = convert(&XmlToJson, &xml);
        let json = first.expect("xml -> json");
        let (back, _) = convert(&JsonToXml, &json);
        let xml_again = back.unwrap_or_else(|error| panic!("{name}: json -> xml: {error}"));
        let (second, events) = convert(&XmlToJson, &xml_again);
        let json_again = second.unwrap_or_else(|error| {
            panic!(
                "{name}: our XML does not read back: {error}\n{}",
                String::from_utf8_lossy(&xml_again)
            )
        });
        assert_eq!(
            String::from_utf8_lossy(&json_again),
            String::from_utf8_lossy(&json),
            "{name}\n{}",
            String::from_utf8_lossy(&xml_again)
        );
        // Our own XML carries no comments, instructions, or doctype; the
        // only loss left is mixed content, which our writer reproduces.
        for loss in losses(&events) {
            assert!(loss.contains("mixed"), "{name}: {loss}");
        }
    }
}

#[test]
fn every_malformed_xml_is_refused() {
    for (name, xml) in fixtures("invalid", "xml") {
        let (result, _) = convert(&XmlToJson, &xml);
        match result {
            Err(ConvertError::Malformed { .. }) => {}
            Err(other) => panic!("{name}: expected Malformed, got {other}"),
            Ok(json) => panic!("{name}: accepted: {}", String::from_utf8_lossy(&json)),
        }
    }
}

#[test]
fn json_writes_the_xml_beside_it_or_is_refused() {
    for (name, json) in fixtures("from-json", "json") {
        let expected_path = fixture_dir("from-json").join(format!("{name}.xml"));
        let (result, _) = convert(&JsonToXml, &json);
        match fs::read(&expected_path) {
            Ok(expected) => {
                let xml = result.unwrap_or_else(|error| panic!("{name}: {error}"));
                assert_eq!(
                    String::from_utf8_lossy(&xml),
                    String::from_utf8_lossy(&expected),
                    "{name}"
                );
            }
            Err(_) => match result {
                Err(ConvertError::Unsupported(_)) => {}
                Err(other) => panic!("{name}: expected Unsupported, got {other}"),
                Ok(xml) => panic!("{name}: accepted: {}", String::from_utf8_lossy(&xml)),
            },
        }
    }
}

#[test]
fn xml_to_json_reports_what_the_mapping_drops() {
    let xml = b"<!-- c --><?p i?><!DOCTYPE r><r><m>a<b/>z</m><m>q<b/></m></r>";
    let (result, events) = convert(&XmlToJson, xml);
    assert_eq!(
        String::from_utf8_lossy(&result.expect("xml -> json")),
        r##"{"r":{"m":[{"b":null,"#text":"a z"},{"b":null,"#text":"q"}]}}"##
    );
    let noted = losses(&events);
    assert_eq!(noted.len(), 4, "{noted:?}");
    assert!(
        noted.iter().any(|loss| loss.contains("comment")),
        "{noted:?}"
    );
    assert!(
        noted.iter().any(|loss| loss.contains("instruction")),
        "{noted:?}"
    );
    assert!(
        noted.iter().any(|loss| loss.contains("doctype")),
        "{noted:?}"
    );
    assert!(
        noted
            .iter()
            .any(|loss| loss.starts_with("r.m:") && loss.contains("mixed")),
        "{noted:?}"
    );
}
