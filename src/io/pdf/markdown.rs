//! A PDF's text as the Markdown event stream, so it reaches Markdown,
//! HTML, and Word through their writers. The structure is read from how
//! the text is set: the most common size is the body; blocks set larger
//! are headings, a level per larger size (three at most); a short block
//! set all in bold at body size is a heading below those. Lines that
//! start with a bullet or a number are list items. A paragraph's lines
//! join with spaces, a word hyphenated across a line end joins whole,
//! and a block that is only a page number is dropped.

use std::borrow::Cow;

use crate::io::markdown::{Event, EventSink, Tag, TagEnd};
use crate::io::pdf::PdfError;
use crate::io::pdf::text::{Block, TextNotes, for_each_page, hyphen_break};

/// Sizes within this many points are one size.
const SIZE_TOLERANCE: f64 = 0.5;
/// A heading is set at least this much larger than the body.
const HEADING_RATIO: f64 = 1.15;
/// A heading is at most this many lines.
const HEADING_LINES: usize = 3;
/// A bold heading at body size is at most this many characters.
const BOLD_HEADING_CHARS: usize = 80;

/// What one block becomes.
enum Kind {
    Heading(u8),
    Paragraph,
    Items { ordered: bool },
}

/// Reads every page (or the one asked for) and pushes the text as
/// Markdown events.
pub fn pdf_events(
    bytes: &[u8],
    page: Option<u32>,
    sink: &mut dyn EventSink<'_>,
) -> Result<TextNotes, PdfError> {
    let mut blocks: Vec<Block> = Vec::new();
    let notes = for_each_page(bytes, page, &mut |_, page_blocks| {
        blocks.extend(
            page_blocks
                .iter()
                .filter(|block| !is_page_number(block))
                .cloned(),
        );
        Ok(())
    })?;
    let body = body_size(&blocks);
    let levels = heading_sizes(&blocks, body);
    let mut open_list: Option<bool> = None;
    for block in &blocks {
        let kind = classify(block, body, &levels);
        let ordered_now = match kind {
            Kind::Items { ordered } => Some(ordered),
            _ => None,
        };
        if open_list.is_some() && open_list != ordered_now {
            let ordered = open_list.take().expect("checked");
            sink.transient(Event::End(TagEnd::List(ordered)));
        }
        match kind {
            Kind::Heading(level) => {
                sink.transient(Event::Start(Tag::Heading(level)));
                sink.transient(Event::Text(Cow::Owned(joined(
                    block.lines.iter().map(|line| line.text.as_str()),
                ))));
                sink.transient(Event::End(TagEnd::Heading(level)));
            }
            Kind::Paragraph => {
                sink.transient(Event::Start(Tag::Paragraph));
                sink.transient(Event::Text(Cow::Owned(joined(
                    block.lines.iter().map(|line| line.text.as_str()),
                ))));
                sink.transient(Event::End(TagEnd::Paragraph));
            }
            Kind::Items { ordered } => {
                if open_list.is_none() {
                    let start = if ordered {
                        block
                            .lines
                            .first()
                            .and_then(|line| number_marker(&line.text))
                            .map(|(number, _)| number)
                    } else {
                        None
                    };
                    sink.transient(Event::Start(Tag::List {
                        start: if ordered {
                            Some(start.unwrap_or(1))
                        } else {
                            None
                        },
                        tight: true,
                    }));
                    open_list = Some(ordered);
                }
                for item in items(block) {
                    sink.transient(Event::Start(Tag::Item));
                    sink.transient(Event::Text(Cow::Owned(item)));
                    sink.transient(Event::End(TagEnd::Item));
                }
            }
        }
    }
    if let Some(ordered) = open_list {
        sink.transient(Event::End(TagEnd::List(ordered)));
    }
    Ok(notes)
}

/// The size most characters are set in.
fn body_size(blocks: &[Block]) -> f64 {
    let mut counts: Vec<(f64, usize)> = Vec::new();
    for line in blocks.iter().flat_map(|block| &block.lines) {
        let characters = line.text.chars().count();
        match counts
            .iter_mut()
            .find(|(size, _)| (size - line.size).abs() <= SIZE_TOLERANCE)
        {
            Some(entry) => entry.1 += characters,
            None => counts.push((line.size, characters)),
        }
    }
    counts
        .iter()
        .max_by_key(|(_, characters)| *characters)
        .map_or(0.0, |(size, _)| *size)
}

/// The distinct sizes of heading-shaped blocks larger than the body,
/// largest first: the first is level 1.
fn heading_sizes(blocks: &[Block], body: f64) -> Vec<f64> {
    let mut sizes: Vec<f64> = Vec::new();
    for block in blocks {
        let size = block_size(block);
        if block.lines.len() <= HEADING_LINES
            && size >= body * HEADING_RATIO
            && !sizes
                .iter()
                .any(|known| (known - size).abs() <= SIZE_TOLERANCE)
        {
            sizes.push(size);
        }
    }
    sizes.sort_by(|a, b| b.total_cmp(a));
    sizes.truncate(3);
    sizes
}

fn block_size(block: &Block) -> f64 {
    block.lines.iter().map(|line| line.size).fold(0.0, f64::max)
}

fn classify(block: &Block, body: f64, levels: &[f64]) -> Kind {
    let first = block.lines.first().map_or("", |line| line.text.as_str());
    // Size first: a numbered section title set large is a heading.
    let size = block_size(block);
    if block.lines.len() <= HEADING_LINES && size >= body * HEADING_RATIO {
        let level = levels
            .iter()
            .position(|known| (known - size).abs() <= SIZE_TOLERANCE)
            .unwrap_or(levels.len())
            + 1;
        return Kind::Heading(level.min(6) as u8);
    }
    if bullet_marker(first).is_some() {
        return Kind::Items { ordered: false };
    }
    if number_marker(first).is_some() {
        return Kind::Items { ordered: true };
    }
    let text_length: usize = block
        .lines
        .iter()
        .map(|line| line.text.chars().count())
        .sum();
    let bold = block.lines.iter().all(|line| line.bold);
    let sentence = first.trim_end().ends_with(['.', ':', ';', ',']);
    if bold && block.lines.len() <= 2 && text_length <= BOLD_HEADING_CHARS && !sentence {
        return Kind::Heading((levels.len() + 1).min(6) as u8);
    }
    Kind::Paragraph
}

/// A leading bullet and the text after it.
fn bullet_marker(line: &str) -> Option<&str> {
    let mut chars = line.chars();
    let first = chars.next()?;
    if !matches!(
        first,
        '•' | '◦' | '▪' | '‣' | '●' | '○' | '■' | '□' | '–' | '-' | '*' | '·'
    ) {
        return None;
    }
    let rest = chars.as_str();
    rest.starts_with(' ').then(|| rest.trim_start())
}

/// A leading `1.` or `1)` and the text after it.
fn number_marker(line: &str) -> Option<(u64, &str)> {
    let digits = line.chars().take_while(char::is_ascii_digit).count();
    if digits == 0 || digits > 3 {
        return None;
    }
    let rest = &line[digits..];
    let rest = rest.strip_prefix('.').or_else(|| rest.strip_prefix(')'))?;
    if !rest.starts_with(' ') {
        return None;
    }
    let number = line[..digits].parse().ok()?;
    Some((number, rest.trim_start()))
}

/// A block's items: each line with a marker starts one, the lines after
/// it continue it.
fn items(block: &Block) -> Vec<String> {
    let mut items: Vec<Vec<&str>> = Vec::new();
    for line in &block.lines {
        let text = line.text.as_str();
        let marked = bullet_marker(text).or_else(|| number_marker(text).map(|(_, rest)| rest));
        match marked {
            Some(rest) => items.push(vec![rest]),
            None => match items.last_mut() {
                Some(item) => item.push(text),
                None => items.push(vec![text]),
            },
        }
    }
    items
        .into_iter()
        .map(|lines| joined(lines.into_iter()))
        .collect()
}

/// Lines joined with spaces; a word broken by a hyphen at a line end
/// (a letter, `-`, then a lowercase letter) joins without it.
fn joined<'a>(lines: impl Iterator<Item = &'a str>) -> String {
    let mut out = String::new();
    for line in lines {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        if out.is_empty() {
            out.push_str(line);
            continue;
        }
        let hyphenated = hyphen_break(&out, line);
        if hyphenated {
            out.pop();
        } else {
            out.push(' ');
        }
        out.push_str(line);
    }
    out
}

/// A block that is only a page number (`12`, `- 12 -`, `Page 12`).
fn is_page_number(block: &Block) -> bool {
    if block.lines.len() != 1 {
        return false;
    }
    let text = block.lines[0].text.trim();
    let text = text.strip_prefix("Page ").unwrap_or(text);
    let text = text.trim_matches(|char: char| char == '-' || char == ' ');
    !text.is_empty() && text.len() <= 4 && text.chars().all(|char| char.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::pdf::text::Line;

    fn line(text: &str, size: f64, bold: bool) -> Line {
        Line {
            text: text.to_string(),
            size,
            bold,
            x: 0.0,
            y: 0.0,
        }
    }

    #[test]
    fn markers_and_joins() {
        assert_eq!(bullet_marker("• item"), Some("item"));
        assert_eq!(bullet_marker("-5 degrees"), None);
        assert_eq!(number_marker("12. twelve"), Some((12, "twelve")));
        assert_eq!(number_marker("2024 was"), None);
        assert_eq!(
            joined(["a hyphen-", "ated word", "and more"].into_iter()),
            "a hyphenated word and more"
        );
        assert_eq!(joined(["Jean-", "Paul"].into_iter()), "Jean- Paul");
        let page = Block {
            lines: vec![line("- 3 -", 9.0, false)],
        };
        assert!(is_page_number(&page));
    }

    #[test]
    fn a_short_bold_block_at_body_size_is_a_heading() {
        let blocks = [
            Block {
                lines: vec![line("Methods", 11.0, true)],
            },
            Block {
                lines: vec![line("We measured it.", 11.0, false)],
            },
        ];
        assert!(matches!(classify(&blocks[0], 11.0, &[]), Kind::Heading(1)));
        assert!(matches!(classify(&blocks[1], 11.0, &[]), Kind::Paragraph));
    }
}
