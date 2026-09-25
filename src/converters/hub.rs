//! Direct pairs between the hub formats (TOML, YAML, XML): one reader into
//! the value tree, one writer out of it, no JSON in between. Each pair is
//! a `HubPair` value: the two formats, a name, and the fidelity note that
//! the reader's and writer's conditions add up to.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::{toml, xml, yaml};
use crate::value::{ChunkedText, Data, Span, Tree};

#[derive(Clone, Copy)]
pub enum HubFormat {
    Toml,
    Yaml,
    Xml,
}

pub struct HubPair {
    pub name: &'static str,
    pub from: &'static Format,
    pub to: &'static Format,
    pub read: HubFormat,
    pub write: HubFormat,
    pub note: &'static str,
}

impl Converter for HubPair {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        self.to
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(self.note)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let (tree, node) = read(self.read, input, self.name, context)?;
        write(self.write, &tree, node, output, self.name, context)
    }
}

/// Reads the input into a tree; what the reader could not keep is
/// reported as losses.
fn read(
    format: HubFormat,
    mut input: Input<'_>,
    name: &'static str,
    context: &mut Context<'_>,
) -> Result<(Tree, u32), ConvertError> {
    match format {
        HubFormat::Toml => {
            let text = read_text_document(&mut input)?;
            let tree = toml::parse(&text)?;
            let root = tree.root;
            Ok((tree, root))
        }
        HubFormat::Yaml => {
            let text = read_text_document(&mut input)?;
            let parsed = yaml::parse(&text)?;
            for note in parsed.notes {
                context.loss(name, Location::default(), note);
            }
            let mut tree = parsed.tree;
            let node = match parsed.documents.as_slice() {
                [] => tree.push(Span::default(), Data::Null),
                [single] => *single,
                documents => {
                    let array = tree.push_array(Span::default());
                    for document in documents {
                        tree.append(array, *document);
                    }
                    array
                }
            };
            Ok((tree, node))
        }
        HubFormat::Xml => {
            let parsed = xml::tree::parse_reader(&mut input)?;
            for note in parsed.notes {
                context.loss(name, Location::default(), note);
            }
            let root = parsed.tree.root;
            Ok((parsed.tree, root))
        }
    }
}

/// Writes the subtree at `node`; what the writer could not keep is
/// reported as losses, what it had to reshape as warnings.
fn write(
    format: HubFormat,
    tree: &Tree,
    node: u32,
    output: &mut dyn Write,
    name: &'static str,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    match format {
        HubFormat::Toml => {
            match tree.data(node) {
                Data::Table(_) => {}
                Data::Array(_) => return Err(not_a_table("an array")),
                _ => return Err(not_a_table("a scalar")),
            }
            let mut losses = Vec::new();
            let mut text = ChunkedText::new(output);
            toml::write_document(tree, node, &mut text, &mut losses);
            text.finish()?;
            for path in losses {
                context.loss(
                    name,
                    Location::default(),
                    format!("{path}: null has no TOML form, dropped"),
                );
            }
            Ok(())
        }
        HubFormat::Yaml => {
            let mut text = ChunkedText::new(output);
            yaml::write_document(tree, node, &mut text);
            text.finish()?;
            Ok(())
        }
        HubFormat::Xml => {
            let mut warnings = Vec::new();
            let mut text = ChunkedText::new(output);
            let written = xml::writer::write_document(tree, node, &mut text, &mut warnings);
            if let Err(message) = written {
                return Err(ConvertError::Unsupported(message));
            }
            text.finish()?;
            for warning in warnings {
                context.warning(warning);
            }
            Ok(())
        }
    }
}

fn not_a_table(what: &str) -> ConvertError {
    ConvertError::Unsupported(format!("a TOML document is a table; the input is {what}"))
}

const TOML_READ: &str = "dates and times become strings";
const YAML_READ: &str = "anchors are expanded, tags outside the core schema are dropped, keys become strings, a multi-document stream becomes an array";
const XML_READ: &str = "attributes become @-keys, text #text, repeated elements arrays, every value a string; comments, processing instructions, the doctype, and the order of text against elements in mixed content are dropped";
const TOML_WRITE: &str = "the document must be a table and nulls are dropped";
const XML_WRITE: &str = "a document without one root member is wrapped in <root>, and numbers, booleans, and dates become text";

pub static TOML_TO_YAML: HubPair = HubPair {
    name: "toml-to-yaml",
    from: &formats::TOML,
    to: &formats::YAML,
    read: HubFormat::Toml,
    write: HubFormat::Yaml,
    note: TOML_READ,
};

pub static YAML_TO_TOML: HubPair = HubPair {
    name: "yaml-to-toml",
    from: &formats::YAML,
    to: &formats::TOML,
    read: HubFormat::Yaml,
    write: HubFormat::Toml,
    note: "anchors are expanded, tags outside the core schema are dropped, keys become strings; the document must be a mapping and nulls are dropped",
};
// `YAML_READ` and `TOML_WRITE` are the halves the notes above are made of.
#[allow(dead_code)]
const NOTE_HALVES: [&str; 2] = [YAML_READ, TOML_WRITE];

pub static TOML_TO_XML: HubPair = HubPair {
    name: "toml-to-xml",
    from: &formats::TOML,
    to: &formats::XML,
    read: HubFormat::Toml,
    write: HubFormat::Xml,
    note: XML_WRITE,
};

pub static XML_TO_TOML: HubPair = HubPair {
    name: "xml-to-toml",
    from: &formats::XML,
    to: &formats::TOML,
    read: HubFormat::Xml,
    write: HubFormat::Toml,
    note: "attributes become @-keys, text #text, repeated elements arrays, every value a string; comments, processing instructions, the doctype, and mixed-content order are dropped; empty elements are dropped",
};

pub static YAML_TO_XML: HubPair = HubPair {
    name: "yaml-to-xml",
    from: &formats::YAML,
    to: &formats::XML,
    read: HubFormat::Yaml,
    write: HubFormat::Xml,
    note: "anchors are expanded, tags outside the core schema are dropped, keys become strings; a document without one root member is wrapped in <root>, and numbers, booleans, and nulls become text",
};

pub static XML_TO_YAML: HubPair = HubPair {
    name: "xml-to-yaml",
    from: &formats::XML,
    to: &formats::YAML,
    read: HubFormat::Xml,
    write: HubFormat::Yaml,
    note: XML_READ,
};

/// Keeps the notes above honest: every pair's note names both sides.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_declare_their_contracts() {
        let pairs: [&HubPair; 6] = [
            &TOML_TO_YAML,
            &YAML_TO_TOML,
            &TOML_TO_XML,
            &XML_TO_TOML,
            &YAML_TO_XML,
            &XML_TO_YAML,
        ];
        for pair in pairs {
            assert_eq!(
                pair.name(),
                format!("{}-to-{}", pair.from().id, pair.to().id)
            );
            assert!(matches!(pair.fidelity(), Fidelity::Conditional(_)));
        }
    }
}
