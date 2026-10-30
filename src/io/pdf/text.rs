//! A page's glyphs as text: lines of glyphs sharing a baseline, words
//! where a gap is wider than a space would be, and blocks of lines split
//! where the line spacing jumps or the size changes. Reading order is the
//! order the page draws its text in, which is reading order for the
//! producers that matter (word processors, LaTeX, browsers); a page that
//! draws its text out of order reads out of order.
//!
//! Japanese written top to bottom reads as columns: a vertical font's
//! glyphs, or Japanese characters set one under another in one column
//! (as the Mac's text system places them), make one line per column, and
//! columns side by side one block, in the order they are drawn (right to
//! left, as they are read).

use std::io::Write;

use crate::io::pdf::PdfError;
use crate::io::pdf::content::{Glyph, Interpreter, PageText};
use crate::io::pdf::document::Document;
use crate::io::pdf::font::FontCache;
use crate::io::pdf::reader::page_content;

/// A line of text and how it was set.
#[derive(Debug, Clone)]
pub struct Line {
    pub text: String,
    /// The largest glyph size on the line, in points.
    pub size: f64,
    /// A column of vertical text: `x` is the column, `y` its top.
    pub vertical: bool,
    /// Every glyph on the line is bold.
    pub bold: bool,
    pub x: f64,
    pub y: f64,
}

/// Lines that belong together: a paragraph, a heading, a list item.
#[derive(Debug, Clone, Default)]
pub struct Block {
    pub lines: Vec<Line>,
}

/// What reading the text left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct TextNotes {
    pub pages: usize,
    /// Pages whose content drew no text (scans, drawings).
    pub pages_without_text: usize,
    /// Glyphs set along a slant or vertically, read in drawing order.
    pub rotated_glyphs: usize,
}

/// A gap wider than this share of the font size is a word break.
const WORD_GAP: f64 = 0.15;
/// A baseline within this share of the size is the same line.
const SAME_LINE: f64 = 0.5;
/// A line more than this many sizes below the last starts a new block.
const BLOCK_GAP: f64 = 1.6;

/// Groups a page's glyphs into blocks of lines.
pub fn blocks(page: &PageText) -> Vec<Block> {
    let mut lines: Vec<(Line, bool)> = Vec::new();
    let mut last: Option<&Glyph> = None;
    for glyph in &page.glyphs {
        let text = &page.text[glyph.text.clone()];
        let down = last.is_some_and(|previous| {
            let open_vertical = lines.last().is_some_and(|(line, _)| line.vertical);
            let open_single = lines
                .last()
                .is_some_and(|(line, _)| line.text.chars().count() <= 1);
            column_follows(previous, glyph, &page.text[previous.text.clone()], text)
                && (open_vertical || open_single)
        });
        if down {
            let (line, all_bold) = lines.last_mut().expect("a line is open");
            line.vertical = true;
            line.text.push_str(text);
            line.size = line.size.max(glyph.size);
            *all_bold &= glyph.bold;
            last = Some(glyph);
            continue;
        }
        let continues = last.is_some_and(|previous| {
            let size = previous.size.max(glyph.size);
            !previous.vertical
                && !glyph.vertical
                && (previous.y - glyph.y).abs() < SAME_LINE * size
                && glyph.x > previous.x - SAME_LINE * size
        });
        if continues {
            let previous = last.expect("checked");
            let (line, all_bold) = lines.last_mut().expect("a line is open");
            let gap = glyph.x - previous.end;
            let size = previous.size.max(glyph.size);
            let spaced = line.text.ends_with(' ') || text.starts_with(' ');
            if gap > WORD_GAP * size && !spaced {
                line.text.push(' ');
            }
            line.text.push_str(text);
            line.size = line.size.max(glyph.size);
            *all_bold &= glyph.bold;
        } else {
            lines.push((
                Line {
                    text: text.to_string(),
                    size: glyph.size,
                    vertical: glyph.vertical,
                    bold: glyph.bold,
                    x: glyph.x,
                    y: glyph.y,
                },
                glyph.bold,
            ));
        }
        last = Some(glyph);
    }
    let mut kept: Vec<Line> = Vec::with_capacity(lines.len());
    for (mut line, all_bold) in lines {
        line.bold = all_bold;
        // A space glyph beside a gap wide enough to be one would read
        // as two spaces; runs of spaces collapse to one.
        let mut collapsed = String::with_capacity(line.text.len());
        for word in line.text.split(' ').filter(|word| !word.is_empty()) {
            if !collapsed.is_empty() {
                collapsed.push(' ');
            }
            collapsed.push_str(word);
        }
        line.text = collapsed;
        if !line.text.is_empty() {
            kept.push(line);
        }
    }
    let spacing = common_spacing(&kept);
    let mut blocks: Vec<Block> = Vec::new();
    for (index, line) in kept.iter().enumerate() {
        if index > 0 && (kept[index - 1].vertical || line.vertical) {
            // Columns side by side, the next to the left, at about the
            // same top.
            let before = &kept[index - 1];
            let size = before.size.max(line.size);
            let step = before.x - line.x;
            let similar =
                (before.size - line.size).abs() <= 0.15 * size && before.bold == line.bold;
            let beside = before.vertical
                && line.vertical
                && similar
                && step > 0.0
                && step < 2.5 * size
                && (before.y - line.y).abs() < 3.0 * size;
            if beside {
                blocks
                    .last_mut()
                    .expect("a block is open")
                    .lines
                    .push(line.clone());
            } else {
                blocks.push(Block {
                    lines: vec![line.clone()],
                });
            }
            continue;
        }
        let joins = index > 0 && {
            let above = &kept[index - 1];
            let drop = above.y - line.y;
            let size = above.size.max(line.size);
            let similar = (above.size - line.size).abs() <= 0.15 * size && above.bold == line.bold;
            // Close by the size, or at the page's usual line spacing: a
            // font whose glyph units are not the usual em (some rewritten
            // CID fonts) states a size that is not the size drawn.
            let close =
                drop < BLOCK_GAP * size || spacing.is_some_and(|usual| drop <= usual * 1.15);
            similar && drop > 0.0 && close
        };
        if joins {
            blocks
                .last_mut()
                .expect("a block is open")
                .lines
                .push(line.clone());
        } else {
            blocks.push(Block {
                lines: vec![line.clone()],
            });
        }
    }
    blocks
}

/// Whether `glyph` is the next one down the column `previous` stands in:
/// both from a vertical font, or both Japanese (or full width) and set one
/// under the other, about a size apart, in the same column.
fn column_follows(previous: &Glyph, glyph: &Glyph, previous_text: &str, text: &str) -> bool {
    let size = previous.size.max(glyph.size);
    let same_column = (previous.x - glyph.x).abs() < 0.3 * size;
    if previous.vertical && glyph.vertical {
        // The next glyph starts about where the last one ended.
        return same_column && glyph.y <= previous.y && (glyph.y - previous.end).abs() < 1.5 * size;
    }
    if previous.vertical != glyph.vertical {
        return false;
    }
    let drop = previous.y - glyph.y;
    same_column
        && drop > 0.6 * size
        && drop < 1.8 * size
        && is_cjk_text(previous_text)
        && is_cjk_text(text)
}

/// Text that is Japanese or full width: kana, ideographs, their
/// punctuation, and the full- and half-width forms.
fn is_cjk_text(text: &str) -> bool {
    let mut chars = text.chars().filter(|ch| !ch.is_whitespace()).peekable();
    chars.peek().is_some() && chars.all(is_cjk_char)
}

/// A Japanese or full-width character (one written without spaces
/// between words).
pub fn is_cjk_char(ch: char) -> bool {
    matches!(u32::from(ch),
        0x3000..=0x30FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE1F
        | 0xFE30..=0xFE4F
        | 0xFF00..=0xFFEF
        | 0x20000..=0x3134F)
}

/// The most common drop from one line to the next among lines of one
/// size (to half a point), the smaller on a tie; `None` when no two lines
/// follow each other down the page.
fn common_spacing(lines: &[Line]) -> Option<f64> {
    let mut counts: Vec<(i64, usize)> = Vec::new();
    for pair in lines.windows(2) {
        let (above, line) = (&pair[0], &pair[1]);
        let drop = above.y - line.y;
        let size = above.size.max(line.size);
        if drop <= 0.0 || (above.size - line.size).abs() > 0.15 * size || drop > 4.0 * size {
            continue;
        }
        let key = (drop * 2.0).round() as i64;
        match counts.iter_mut().find(|entry| entry.0 == key) {
            Some(entry) => entry.1 += 1,
            None => counts.push((key, 1)),
        }
    }
    counts
        .iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(&a.0)))
        .filter(|entry| entry.1 >= 2)
        .map(|entry| entry.0 as f64 / 2.0)
}

/// What receives each page: its index and its blocks.
pub type PageHandler<'h> = dyn FnMut(usize, &[Block]) -> Result<(), PdfError> + 'h;

/// Runs every page (or the one asked for) and hands each page's blocks
/// to `each`, in order.
pub fn for_each_page(
    bytes: &[u8],
    page: Option<u32>,
    each: &mut PageHandler<'_>,
) -> Result<TextNotes, PdfError> {
    let document = Document::open(bytes)?;
    let pages = document.pages()?;
    let mut notes = TextNotes {
        pages: pages.len(),
        ..TextNotes::default()
    };
    let chosen: Vec<usize> = match page {
        None => (0..pages.len()).collect(),
        Some(number) if number >= 1 && (number as usize) <= pages.len() => {
            vec![number as usize - 1]
        }
        Some(number) => {
            return Err(PdfError(format!(
                "the PDF has {} pages; there is no page {number}",
                pages.len()
            )));
        }
    };
    let mut fonts = FontCache::default();
    for index in chosen {
        let page = &pages[index];
        let content = page_content(&document, &page.dictionary);
        let text = Interpreter::new(&document, &mut fonts).run(&content, page.resources.as_ref());
        notes.rotated_glyphs += text.glyphs.iter().filter(|glyph| !glyph.upright).count();
        let page_blocks = blocks(&text);
        if page_blocks.is_empty() {
            notes.pages_without_text += 1;
        }
        each(index, &page_blocks)?;
    }
    Ok(notes)
}

/// Whether `line` continues a word `before` broke with a hyphen: a
/// letter, then `-` at the end, and a lowercase letter to start `line`.
pub fn hyphen_break(before: &str, line: &str) -> bool {
    let Some(stem) = before.strip_suffix('-') else {
        return false;
    };
    stem.chars().last().is_some_and(char::is_alphabetic)
        && line.chars().next().is_some_and(char::is_lowercase)
}

/// Writes the text as pdftotext lays it out without `-layout`: a line
/// per line, a blank line after each block, a form feed after each page.
pub fn write_pdf_text(
    bytes: &[u8],
    page: Option<u32>,
    out: &mut dyn Write,
) -> Result<TextNotes, PdfError> {
    let io = |error: std::io::Error| PdfError(format!("writing the text: {error}"));
    for_each_page(bytes, page, &mut |_, blocks| {
        for block in blocks {
            let mut pending = String::new();
            for line in &block.lines {
                // A word hyphenated across a line end joins the next line,
                // as pdftotext joins it.
                if hyphen_break(&pending, &line.text) {
                    pending.pop();
                    pending.push_str(&line.text);
                    continue;
                }
                if !pending.is_empty() {
                    out.write_all(pending.as_bytes()).map_err(io)?;
                    out.write_all(b"\n").map_err(io)?;
                }
                pending.clone_from(&line.text);
            }
            out.write_all(pending.as_bytes()).map_err(io)?;
            out.write_all(b"\n\n").map_err(io)?;
        }
        out.write_all(b"\x0c").map_err(io)
    })
}
