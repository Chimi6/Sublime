//! A page's glyphs as text: lines of glyphs sharing a baseline, words
//! where a gap is wider than a space would be, and blocks of lines split
//! where the line spacing jumps or the size changes. Reading order is the
//! order the page draws its text in, which is reading order for the
//! producers that matter (word processors, LaTeX, browsers); a page that
//! draws its text out of order reads out of order.

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
        let continues = last.is_some_and(|previous| {
            let size = previous.size.max(glyph.size);
            (previous.y - glyph.y).abs() < SAME_LINE * size
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
                    bold: glyph.bold,
                    x: glyph.x,
                    y: glyph.y,
                },
                glyph.bold,
            ));
        }
        last = Some(glyph);
    }
    let mut blocks: Vec<Block> = Vec::new();
    let mut previous: Option<Line> = None;
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
        if line.text.trim().is_empty() {
            continue;
        }
        let joins = previous.as_ref().is_some_and(|above| {
            let drop = above.y - line.y;
            let size = above.size.max(line.size);
            let similar = (above.size - line.size).abs() <= 0.15 * size;
            drop > 0.0 && drop < BLOCK_GAP * size && similar && above.bold == line.bold
        });
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
        previous = Some(line);
    }
    blocks
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
            for line in &block.lines {
                out.write_all(line.text.as_bytes()).map_err(io)?;
                out.write_all(b"\n").map_err(io)?;
            }
            out.write_all(b"\n").map_err(io)?;
        }
        out.write_all(b"\x0c").map_err(io)
    })
}
