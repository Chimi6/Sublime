//! A pull reader over well-formed XML: start and end tags with attributes,
//! text with entities decoded (whitespace-only text included: a Word run
//! of one space is text), and nothing else (comments, processing
//! instructions, and the prolog are skipped; CDATA is text). Enough for
//! the Office and OpenDocument formats, which are machine-written.

use std::borrow::Cow;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum XmlEvent<'a> {
    /// `<name attr="value">`, with `self_closing` for `<name/>`.
    Start {
        name: &'a str,
        attributes: Vec<(&'a str, Cow<'a, str>)>,
        self_closing: bool,
    },
    End {
        name: &'a str,
    },
    Text(Cow<'a, str>),
}

pub struct XmlReader<'a> {
    text: &'a str,
    position: usize,
}

impl<'a> XmlReader<'a> {
    pub fn new(text: &'a str) -> XmlReader<'a> {
        XmlReader { text, position: 0 }
    }

    /// Text between the next start tag of `name` and its end tag, with
    /// nested elements' text concatenated.
    pub fn attribute<'e>(event: &'e XmlEvent<'a>, name: &str) -> Option<&'e str> {
        match event {
            XmlEvent::Start { attributes, .. } => attributes
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_ref()),
            _ => None,
        }
    }
}

impl<'a> Iterator for XmlReader<'a> {
    type Item = XmlEvent<'a>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            let rest = &self.text[self.position..];
            if rest.is_empty() {
                return None;
            }
            if let Some(after) = rest.strip_prefix('<') {
                if after.starts_with("!--") {
                    let end = rest.find("-->").map_or(rest.len(), |index| index + 3);
                    self.position += end;
                    continue;
                }
                if let Some(cdata) = after.strip_prefix("![CDATA[") {
                    let end = cdata.find("]]>").unwrap_or(cdata.len());
                    let text = &cdata[..end];
                    self.position += 9 + end + 3;
                    return Some(XmlEvent::Text(Cow::Borrowed(text)));
                }
                if after.starts_with('?') || after.starts_with('!') {
                    let end = rest.find('>').map_or(rest.len(), |index| index + 1);
                    self.position += end;
                    continue;
                }
                let end = rest.find('>').unwrap_or(rest.len() - 1);
                let tag = &rest[1..end];
                self.position += end + 1;
                if let Some(name) = tag.strip_prefix('/') {
                    return Some(XmlEvent::End { name: name.trim() });
                }
                let self_closing = tag.ends_with('/');
                let tag = tag.trim_end_matches('/');
                let (name, attributes) = parse_tag(tag);
                return Some(XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                });
            }
            let end = rest.find('<').unwrap_or(rest.len());
            let text = &rest[..end];
            self.position += end;
            return Some(XmlEvent::Text(decode_entities(text)));
        }
    }
}

fn parse_tag(tag: &str) -> (&str, Vec<(&str, Cow<'_, str>)>) {
    let name_end = tag.find(char::is_whitespace).unwrap_or(tag.len());
    let name = &tag[..name_end];
    let mut attributes = Vec::new();
    let mut rest = tag[name_end..].trim_start();
    while !rest.is_empty() {
        let equals = match rest.find('=') {
            Some(index) => index,
            None => break,
        };
        let key = rest[..equals].trim();
        let after = rest[equals + 1..].trim_start();
        let quote = match after.chars().next() {
            Some(quote @ ('"' | '\'')) => quote,
            _ => break,
        };
        let value_end = match after[1..].find(quote) {
            Some(index) => index + 1,
            None => break,
        };
        let value = &after[1..value_end];
        attributes.push((key, decode_entities(value)));
        rest = after[value_end + 1..].trim_start();
    }
    (name, attributes)
}

/// Decodes the predefined and numeric character references.
pub fn decode_entities(text: &str) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.find('&') {
        out.push_str(&rest[..index]);
        let after = &rest[index + 1..];
        let end = match after.find(';') {
            Some(end) if end <= 10 => end,
            _ => {
                out.push('&');
                rest = after;
                continue;
            }
        };
        let entity = &after[..end];
        let decoded = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix('#')
                .and_then(|number| match number.strip_prefix('x') {
                    Some(hex) => u32::from_str_radix(hex, 16).ok(),
                    None => number.parse::<u32>().ok(),
                })
                .and_then(char::from_u32),
        };
        match decoded {
            Some(ch) => out.push(ch),
            None => {
                out.push('&');
                out.push_str(entity);
                out.push(';');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Cow::Owned(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tags_text_and_entities() {
        let events: Vec<XmlEvent<'_>> = XmlReader::new(
            r#"<?xml version="1.0"?><a x="1" y='&lt;'><b/>hi &amp; bye<!-- c --></a>"#,
        )
        .collect();
        assert_eq!(events.len(), 4);
        assert!(
            matches!(&events[0], XmlEvent::Start { name: "a", attributes, self_closing: false } if attributes[1].1 == "<")
        );
        assert!(matches!(
            &events[1],
            XmlEvent::Start {
                name: "b",
                self_closing: true,
                ..
            }
        ));
        assert_eq!(
            events[2],
            XmlEvent::Text(Cow::Owned("hi & bye".to_string()))
        );
        assert_eq!(events[3], XmlEvent::End { name: "a" });
    }
}
