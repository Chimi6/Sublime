//! The one list of converters. Adding a converter is one line here plus one
//! `pub mod` line in `src/converters/mod.rs`.

use crate::converter::Converter;
use crate::converters::csv_to_json::CsvToJson;
use crate::converters::json_to_csv::JsonToCsv;
use crate::format::Format;

static CONVERTERS: [&dyn Converter; 2] = [&CsvToJson, &JsonToCsv];

pub fn all_converters() -> &'static [&'static dyn Converter] {
    &CONVERTERS
}

/// Every format referenced by a registered converter, unique, sorted by id.
pub fn all_formats() -> Vec<&'static Format> {
    let mut formats: Vec<&'static Format> = Vec::new();
    for converter in all_converters() {
        push_unique(&mut formats, converter.from());
        push_unique(&mut formats, converter.to());
    }
    formats.sort_by(|left, right| left.id.cmp(right.id));
    formats
}

fn push_unique(formats: &mut Vec<&'static Format>, format: &'static Format) {
    let already_present = formats.iter().any(|existing| existing.id == format.id);
    if !already_present {
        formats.push(format);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn converter_names_are_unique() {
        let mut names = HashSet::new();
        for converter in all_converters() {
            let is_new = names.insert(converter.name());
            assert!(is_new, "duplicate converter name {}", converter.name());
        }
    }

    #[test]
    fn edges_are_unique() {
        let mut edges = HashSet::new();
        for converter in all_converters() {
            let edge = (converter.from().id, converter.to().id);
            let is_new = edges.insert(edge);
            assert!(is_new, "duplicate edge {:?}", edge);
        }
    }

    #[test]
    fn every_format_has_an_extension() {
        for format in all_formats() {
            assert!(
                !format.extensions.is_empty(),
                "{} has no extensions",
                format.id
            );
        }
    }

    #[test]
    fn formats_are_sorted_and_unique() {
        let formats = all_formats();
        let ids: Vec<&str> = formats.iter().map(|format| format.id).collect();
        assert_eq!(ids, vec!["csv", "json"]);
    }

    #[test]
    fn format_ids_are_lowercase() {
        for format in all_formats() {
            let lowered = format.id.to_ascii_lowercase();
            assert_eq!(format.id, lowered, "{} must be lowercase", format.id);
        }
    }
}
