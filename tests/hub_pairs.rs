//! The direct pairs between TOML, YAML, and XML produce the same document
//! the two-hop path through JSON produces (compared as JSON, since the
//! direct path keeps infinities and NaN as floats where JSON cannot), on
//! every fixture of the source format, and refuse exactly what it refuses.

use std::fs;
use std::path::PathBuf;

use sublime::converter::{ConvertError, ConvertOptions, Converter, Input};
use sublime::converters::hub;
use sublime::converters::json_to_toml::JsonToToml;
use sublime::converters::json_to_xml::JsonToXml;
use sublime::converters::json_to_yaml::JsonToYaml;
use sublime::converters::toml_to_json::TomlToJson;
use sublime::converters::xml_to_json::XmlToJson;
use sublime::converters::yaml_to_json::YamlToJson;
use sublime::event::{Context, NullSink};

fn convert(converter: &dyn Converter, bytes: &[u8]) -> Result<Vec<u8>, ConvertError> {
    let options = ConvertOptions::default();
    let mut sink = NullSink;
    let mut context = Context::new(&mut sink, &options);
    let mut output = Vec::new();
    let mut cursor = std::io::Cursor::new(bytes);
    converter.convert(Input::Rewindable(&mut cursor), &mut output, &mut context)?;
    Ok(output)
}

fn fixtures(format: &str) -> Vec<(String, Vec<u8>)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .join(format);
    let mut paths: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("fixture dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == format))
        .collect();
    paths.sort();
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

/// Every source fixture through the direct pair and through JSON; the two
/// outputs are read back to JSON with `target_to_json` and compared.
fn check(
    format: &str,
    direct: &dyn Converter,
    to_json: &dyn Converter,
    from_json: &dyn Converter,
    target_to_json: &dyn Converter,
) {
    for (name, source) in fixtures(format) {
        // Infinities and NaN are floats on the direct path and strings on
        // the JSON route; the dedicated test below pins their spelling.
        let carries_infinities = source
            .windows(3)
            .any(|window| window == b"inf" || window == b"nan");
        if carries_infinities {
            continue;
        }
        let json = convert(to_json, &source).expect("source reads");
        let via_json = convert(from_json, &json);
        let straight = convert(direct, &source);
        match (straight, via_json) {
            (Ok(straight), Ok(via_json)) => {
                let straight_json =
                    convert(target_to_json, &straight).expect("direct output reads back");
                let via_json_json =
                    convert(target_to_json, &via_json).expect("JSON-route output reads back");
                assert_eq!(
                    String::from_utf8_lossy(&straight_json),
                    String::from_utf8_lossy(&via_json_json),
                    "{}: {name}",
                    direct.name()
                );
            }
            (Err(ConvertError::Unsupported(_)), Err(ConvertError::Unsupported(_))) => {}
            (straight, via_json) => panic!(
                "{}: {name}: direct {:?}, via JSON {:?}",
                direct.name(),
                straight.map(|bytes| bytes.len()),
                via_json.map(|bytes| bytes.len())
            ),
        }
    }
}

#[test]
fn toml_pairs_match_the_json_route() {
    check(
        "toml",
        &hub::TOML_TO_YAML,
        &TomlToJson,
        &JsonToYaml,
        &YamlToJson,
    );
    check(
        "toml",
        &hub::TOML_TO_XML,
        &TomlToJson,
        &JsonToXml,
        &XmlToJson,
    );
}

#[test]
fn yaml_pairs_match_the_json_route() {
    check(
        "yaml",
        &hub::YAML_TO_TOML,
        &YamlToJson,
        &JsonToToml,
        &TomlToJson,
    );
    check(
        "yaml",
        &hub::YAML_TO_XML,
        &YamlToJson,
        &JsonToXml,
        &XmlToJson,
    );
}

#[test]
fn xml_pairs_match_the_json_route() {
    check(
        "xml",
        &hub::XML_TO_TOML,
        &XmlToJson,
        &JsonToToml,
        &TomlToJson,
    );
    check(
        "xml",
        &hub::XML_TO_YAML,
        &XmlToJson,
        &JsonToYaml,
        &YamlToJson,
    );
}

/// What the direct pairs keep that the JSON route cannot.
#[test]
fn direct_pairs_keep_infinities_and_nan_as_floats() {
    let yaml = convert(
        &hub::TOML_TO_YAML,
        b"a = inf
b = -inf
c = nan
d = 1979-05-27
",
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&yaml),
        "a: .inf
b: -.inf
c: .nan
d: 1979-05-27
"
    );
    let toml = convert(
        &hub::YAML_TO_TOML,
        b"a: .inf
b: -.inf
c: .NaN
",
    )
    .unwrap();
    assert_eq!(
        String::from_utf8_lossy(&toml),
        "a = inf
b = -inf
c = nan
"
    );
}

#[test]
fn the_planner_takes_the_direct_edge() {
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_sublime"))
        .args(["paths"])
        .output()
        .expect("run sublime");
    let text = String::from_utf8_lossy(&output.stdout);
    assert!(
        text.contains("toml -> yaml: conditional via toml-to-yaml\n"),
        "{text}"
    );
    assert!(
        text.contains("xml -> yaml: conditional via xml-to-yaml\n"),
        "{text}"
    );
}
