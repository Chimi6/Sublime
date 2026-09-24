//! Every Pages fixture is a ZIP of IWA streams. This opens each one with our
//! ZIP reader, verifies every entry's checksum, and checks that the main
//! document stream has the IWA chunk framing.

use std::path::PathBuf;

use sublime::io::iwa::{decompress_stream, parse_objects};
use sublime::io::zip::ZipArchive;

fn fixtures() -> Vec<PathBuf> {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("tests/fixtures/pages");
    let mut paths: Vec<PathBuf> = std::fs::read_dir(&dir)
        .expect("fixture directory")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "pages")
        })
        .collect();
    paths.sort();
    assert!(!paths.is_empty(), "no .pages fixtures found");
    paths
}

#[test]
fn every_fixture_unzips_with_matching_checksums() {
    for path in fixtures() {
        let bytes = std::fs::read(&path).expect("fixture readable");
        let archive =
            ZipArchive::parse(&bytes).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let mut out = Vec::new();
        for entry in archive.entries() {
            if entry.is_directory() {
                continue;
            }
            out.clear();
            archive
                .read(entry, &mut out)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        }
        let document = archive
            .find("Index/Document.iwa")
            .unwrap_or_else(|| panic!("{}: no Index/Document.iwa", path.display()));
        out.clear();
        archive.read(document, &mut out).unwrap();
        assert_eq!(out[0], 0, "{}: IWA chunk type", path.display());
        let declared =
            usize::from(out[1]) | (usize::from(out[2]) << 8) | (usize::from(out[3]) << 16);
        assert!(
            declared <= out.len() - 4,
            "{}: IWA chunk length",
            path.display()
        );
    }
}

#[test]
fn deflated_word_sources_unzip_with_matching_checksums() {
    let mut dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    dir.push("tests/fixtures/pages/sources");
    let mut count = 0usize;
    for entry in std::fs::read_dir(&dir).expect("sources directory") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|extension| extension != "docx") {
            continue;
        }
        let bytes = std::fs::read(&path).expect("source readable");
        let archive = ZipArchive::parse(&bytes).unwrap();
        let mut out = Vec::new();
        for entry in archive.entries() {
            assert_eq!(
                entry.method,
                8,
                "{}: {} should be deflated",
                path.display(),
                entry.name
            );
            out.clear();
            archive
                .read(entry, &mut out)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            count += 1;
        }
    }
    assert!(count > 20);
}

/// Every IWA stream in every fixture decodes into objects; prints the type
/// histogram of the main document stream when run with `--nocapture`.
#[test]
fn every_iwa_stream_parses_into_objects() {
    use std::collections::BTreeMap;
    let mut histogram: BTreeMap<u32, usize> = BTreeMap::new();
    let mut multi = 0usize;
    for path in fixtures() {
        let bytes = std::fs::read(&path).expect("fixture readable");
        let archive = ZipArchive::parse(&bytes).unwrap();
        let mut compressed = Vec::new();
        for entry in archive.entries() {
            if !entry.name.ends_with(".iwa") {
                continue;
            }
            compressed.clear();
            archive.read(entry, &mut compressed).unwrap();
            let stream = decompress_stream(&compressed)
                .unwrap_or_else(|error| panic!("{}: {}: {error}", path.display(), entry.name));
            let objects = parse_objects(&stream)
                .unwrap_or_else(|error| panic!("{}: {}: {error}", path.display(), entry.name));
            assert!(
                !objects.is_empty(),
                "{}: {} has no objects",
                path.display(),
                entry.name
            );
            for object in &objects {
                assert!(
                    !object.messages.is_empty(),
                    "{}: {}: object {} has no messages",
                    path.display(),
                    entry.name,
                    object.identifier
                );
                if object.messages.len() > 1 {
                    multi += 1;
                }
                *histogram
                    .entry(object.message_type().unwrap_or(0))
                    .or_default() += 1;
            }
        }
    }
    let mut types: Vec<(usize, u32)> = histogram
        .into_iter()
        .map(|(kind, count)| (count, kind))
        .collect();
    types.sort_unstable_by(|left, right| right.cmp(left));
    eprintln!(
        "{} distinct message types, {multi} objects with more than one message; most common:",
        types.len()
    );
    for (count, kind) in types.iter().take(40) {
        eprintln!("  type {kind}: {count}");
    }
}
