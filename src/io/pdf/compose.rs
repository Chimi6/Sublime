//! Sets the Markdown event stream as PDF pages: every document reader
//! emits that stream, so every document format reaches PDF through here.
//!
//! Blocks collect their styled runs; a finished block is broken into
//! lines first-fit on the fonts' real widths and set down the page;
//! a page is written the moment the next line will not fit, so memory
//! holds one page. Fonts are the base-14 Helvetica and Courier families
//! (nothing embedded); text is WinAnsi, and a character outside it is
//! set as `?` and counted as a loss.

use std::collections::HashMap;
use std::io;
use std::sync::{Arc, OnceLock};

use crate::io::font::Font;
#[cfg(not(target_arch = "wasm32"))]
use crate::io::font::system::SystemFonts;
use crate::io::markdown::{Alignment, Event, EventSink, Tag, TagEnd};
use crate::io::pdf::tables::{self, Widths};
use crate::io::pdf::writer::{
    ImageObject, PdfDocument, StandardFont, TextPage, literal, number_text,
};
use crate::io::png::reader::RowSink;

/// A picture's bytes and, when the document states it, its size in points.
pub type Picture = (Vec<u8>, Option<(f64, f64)>);

/// Where a document's images come from. `resolve` turns an image's
/// destination (a media file name, a path, a `data:` URI) into the
/// picture's bytes and, when the document states it, its size in points;
/// `decode` reads a picture that is not a JPEG into rows.
pub trait ImageSource {
    fn resolve(&mut self, destination: &str) -> Option<Picture>;
    fn decode(&mut self, bytes: &[u8], sink: &mut dyn RowSink) -> io::Result<()>;
}

/// An image's size in points: by the resolution it records, else at 96
/// pixels to the inch.
fn natural_size(object: &ImageObject) -> (f64, f64) {
    match object.density {
        Some((across, down)) => (
            f64::from(object.width) * 72.0 / across,
            f64::from(object.height) * 72.0 / down,
        ),
        None => (
            f64::from(object.width) * POINTS_PER_PIXEL,
            f64::from(object.height) * POINTS_PER_PIXEL,
        ),
    }
}

/// Points per pixel for an image that records no resolution: 96 pixels
/// to the inch, as a browser shows it.
const POINTS_PER_PIXEL: f64 = 0.75;

/// A page and its margins, in points.
#[derive(Debug, Clone, Copy)]
pub struct PageSetup {
    pub width: f64,
    pub height: f64,
    pub margin: f64,
}

impl Default for PageSetup {
    /// US Letter with one-inch margins.
    fn default() -> PageSetup {
        PageSetup {
            width: 612.0,
            height: 792.0,
            margin: 72.0,
        }
    }
}

/// What setting the document left out.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct ComposeNotes {
    pub pages: usize,
    /// Characters outside WinAnsi, set as `?`.
    pub unset_characters: usize,
    /// Images shown by their alt text: not found, or not readable.
    pub images: usize,
    /// Images set on the page.
    pub images_placed: usize,
    /// Raw HTML, dropped.
    pub html: usize,
}

const BODY: f64 = 11.0;
const BODY_LEADING: f64 = 15.0;
const HEADING_SIZES: [f64; 6] = [20.0, 16.0, 13.0, 11.0, 11.0, 11.0];
const CODE_SIZE: f64 = 9.5;
const CODE_LEADING: f64 = 12.5;
const BLOCK_SPACE: f64 = 8.0;
const LIST_INDENT: f64 = 18.0;
const QUOTE_INDENT: f64 = 16.0;
const CELL_PAD: f64 = 4.0;
const LINK_COLOR: &str = "0 0 0.75 rg";

#[derive(Debug, Clone, Default, PartialEq)]
struct Style {
    bold: bool,
    italic: bool,
    code: bool,
    link: Option<String>,
}

impl Style {
    fn font(&self) -> StandardFont {
        match (self.code, self.bold, self.italic) {
            (true, true, _) => StandardFont::CourierBold,
            (true, false, _) => StandardFont::Courier,
            (false, true, true) => StandardFont::HelveticaBoldOblique,
            (false, true, false) => StandardFont::HelveticaBold,
            (false, false, true) => StandardFont::HelveticaOblique,
            (false, false, false) => StandardFont::Helvetica,
        }
    }
}

/// Where a character is set from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Face {
    /// The style's base-14 font (the character is WinAnsi).
    Standard,
    /// An embedded font, by its slot in `Faces::fonts`.
    Embedded(usize),
    /// No font has it: set as `?` and counted.
    Missing,
}

/// The fonts text is set in beyond the base-14: a font the caller chose
/// for all body text, and fonts found on this machine for the characters
/// the standard fonts lack.
pub struct Faces {
    forced: Option<usize>,
    fonts: Vec<Arc<Font>>,
    /// Each slot's index in the document's embedded fonts, once used.
    embedded: Vec<Option<usize>>,
    #[cfg(not(target_arch = "wasm32"))]
    system: Option<SystemFonts>,
    cache: HashMap<(char, bool), Face>,
}

impl Faces {
    /// `forced` sets all body text (not code); `system` searches the
    /// machine's fonts for characters nothing else covers.
    pub fn new(forced: Option<Arc<Font>>, system: bool) -> Faces {
        let mut fonts = Vec::new();
        let forced = forced.map(|font| {
            fonts.push(font);
            0
        });
        let embedded = vec![None; fonts.len()];
        #[cfg(target_arch = "wasm32")]
        let _ = system;
        Faces {
            forced,
            fonts,
            embedded,
            #[cfg(not(target_arch = "wasm32"))]
            system: system.then(SystemFonts::default),
            cache: HashMap::new(),
        }
    }

    /// Only the base-14 fonts: characters outside WinAnsi are `?`.
    pub fn standard() -> Faces {
        Faces::new(None, false)
    }

    fn face(&mut self, char: char, code: bool) -> Face {
        if let Some(face) = self.cache.get(&(char, code)) {
            return *face;
        }
        let face = self.resolve(char, code);
        self.cache.insert((char, code), face);
        face
    }

    fn resolve(&mut self, char: char, code: bool) -> Face {
        if !code
            && let Some(slot) = self.forced
            && self.fonts[slot].glyph(char).is_some()
        {
            return Face::Embedded(slot);
        }
        if win_ansi_byte(char).is_some() {
            return Face::Standard;
        }
        if let Some(slot) = self
            .fonts
            .iter()
            .position(|font| font.glyph(char).is_some())
        {
            return Face::Embedded(slot);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(system) = &mut self.system
            && let Some((_, font)) = system.font_for(char)
        {
            let slot = match self
                .fonts
                .iter()
                .position(|known| Arc::ptr_eq(known, &font))
            {
                Some(slot) => slot,
                None => {
                    self.fonts.push(font);
                    self.embedded.push(None);
                    self.fonts.len() - 1
                }
            };
            return Face::Embedded(slot);
        }
        Face::Missing
    }

    /// The width of `text` in `style` at `size`.
    fn width(&mut self, style: &Style, size: f64, text: &str) -> f64 {
        let mut total = 0.0;
        for char in text.chars() {
            total += match self.face(char, style.code) {
                Face::Standard => char_width(style.font(), size, char),
                Face::Embedded(slot) => {
                    let font = &self.fonts[slot];
                    let glyph = font.glyph(char).unwrap_or(0);
                    font.to_thousandths(f64::from(font.advance(glyph))) / 1000.0 * size
                }
                Face::Missing => char_width(style.font(), size, '?'),
            };
        }
        total
    }
}

/// Styled text inside a block; `"\n"` alone is a hard line break.
#[derive(Debug, Clone)]
struct Run {
    text: String,
    style: Style,
}

/// A piece of a set line: text in one style at an x offset.
#[derive(Debug, Clone)]
struct Piece {
    text: String,
    style: Style,
    x: f64,
    width: f64,
}

type Line = Vec<Piece>;

struct List {
    /// The next number, for an ordered list.
    next: Option<u64>,
    /// Items without paragraphs set close together.
    tight: bool,
}

#[derive(Default)]
struct TableBuild {
    alignments: Vec<Alignment>,
    rows: Vec<(bool, Vec<Vec<Run>>)>,
    in_head: bool,
}

pub struct Composer<'w, 'a> {
    document: &'w mut PdfDocument<'a>,
    setup: PageSetup,
    // The page being set.
    content: String,
    fonts: Vec<StandardFont>,
    links: Vec<([f64; 4], String)>,
    /// The top of the next line, in page space.
    top: f64,
    page_has_content: bool,
    pending_space: f64,
    // The block being collected.
    runs: Vec<Run>,
    bold: usize,
    italic: usize,
    links_open: Vec<String>,
    heading: Option<u8>,
    heading_text: String,
    title_taken: bool,
    lists: Vec<List>,
    marker: Option<String>,
    quotes: usize,
    code: Option<String>,
    table: Option<TableBuild>,
    image_depth: usize,
    /// Images open whose alt text is not set, because the picture is.
    placed_depth: usize,
    /// Where the document's images come from, when it has a source.
    images: Option<&'w mut dyn ImageSource>,
    /// Image objects the page being set shows, as `/I1`, `/I2`, ...
    page_images: Vec<u32>,
    pub notes: ComposeNotes,
    error: Option<io::Error>,
    faces: Faces,
    /// Embedded fonts the page being set uses, by document index.
    page_embedded: Vec<usize>,
}

impl<'w, 'a> Composer<'w, 'a> {
    pub fn new(
        document: &'w mut PdfDocument<'a>,
        setup: PageSetup,
        faces: Faces,
    ) -> Composer<'w, 'a> {
        Composer {
            document,
            setup,
            content: String::new(),
            fonts: Vec::new(),
            links: Vec::new(),
            top: setup.height - setup.margin,
            page_has_content: false,
            pending_space: 0.0,
            runs: Vec::new(),
            bold: 0,
            italic: 0,
            links_open: Vec::new(),
            heading: None,
            heading_text: String::new(),
            title_taken: false,
            lists: Vec::new(),
            marker: None,
            quotes: 0,
            code: None,
            table: None,
            image_depth: 0,
            placed_depth: 0,
            images: None,
            page_images: Vec::new(),
            notes: ComposeNotes::default(),
            error: None,
            faces,
            page_embedded: Vec::new(),
        }
    }

    /// Sets the document's images from `source`; without one, images are
    /// shown by their alt text.
    pub fn with_images(mut self, source: &'w mut dyn ImageSource) -> Composer<'w, 'a> {
        self.images = Some(source);
        self
    }

    /// Writes the last page (an empty document still gets one).
    pub fn finish(mut self) -> io::Result<ComposeNotes> {
        self.flush_runs();
        if self.page_has_content || self.notes.pages == 0 {
            self.end_page();
        }
        match self.error.take() {
            Some(error) => Err(error),
            None => Ok(self.notes),
        }
    }

    fn style(&self) -> Style {
        Style {
            bold: self.bold > 0 || self.heading.is_some(),
            italic: self.italic > 0,
            code: false,
            link: self.links_open.last().cloned(),
        }
    }

    fn push_text(&mut self, text: &str, code: bool) {
        if self.placed_depth > 0 {
            return;
        }
        let mut style = self.style();
        style.code = code;
        if let Some(table) = &mut self.table {
            if let Some((_, cells)) = table.rows.last_mut()
                && let Some(cell) = cells.last_mut()
            {
                cell.push(Run {
                    text: text.to_string(),
                    style,
                });
            }
            return;
        }
        if self.heading.is_some() && !self.title_taken {
            self.heading_text.push_str(text);
        }
        self.runs.push(Run {
            text: text.to_string(),
            style,
        });
    }

    /// The left edge of text at the current nesting.
    fn left(&self) -> f64 {
        self.setup.margin
            + self.lists.len() as f64 * LIST_INDENT
            + self.quotes as f64 * QUOTE_INDENT
    }

    fn right(&self) -> f64 {
        self.setup.width - self.setup.margin
    }

    // ------------------------------------------------------------ pages

    fn end_page(&mut self) {
        if self.error.is_some() {
            return;
        }
        let page = TextPage {
            width: self.setup.width,
            height: self.setup.height,
            content: self.content.as_bytes(),
            fonts: &self.fonts,
            embedded: &self.page_embedded,
            links: &self.links,
            images: &self.page_images,
        };
        if let Err(error) = self.document.text_page(&page) {
            self.error = Some(error);
        }
        self.notes.pages += 1;
        self.content.clear();
        self.fonts.clear();
        self.page_embedded.clear();
        self.page_images.clear();
        self.links.clear();
        self.top = self.setup.height - self.setup.margin;
        self.page_has_content = false;
    }

    /// Makes room for `height` more points of lines, starting a page
    /// when they would pass the bottom margin; spacing owed from the
    /// block before is dropped at the top of a page.
    fn room(&mut self, height: f64) {
        let spacing = if self.page_has_content {
            self.pending_space
        } else {
            0.0
        };
        if self.page_has_content && self.top - spacing - height < self.setup.margin {
            self.end_page();
            self.pending_space = 0.0;
            return;
        }
        self.top -= spacing;
        self.pending_space = 0.0;
    }

    /// Sets an image from the document's source as a block of its own,
    /// at its size (scaled down to the text width and the page), on a new
    /// page when it does not fit. False when there is no source, the
    /// picture is not found, or it does not read.
    fn place_image(&mut self, destination: &str) -> bool {
        let Some(source) = self.images.as_deref_mut() else {
            return false;
        };
        let Some((bytes, size)) = source.resolve(destination) else {
            return false;
        };
        let object = if bytes.starts_with(&[0xFF, 0xD8]) {
            self.document.jpeg_image(&bytes).ok()
        } else {
            // Read whole first: a picture that fails partway leaves no
            // half-written object behind.
            let mut collect = crate::io::png::reader::Collect::default();
            if source.decode(&bytes, &mut collect).is_err() || collect.image.height == 0 {
                return false;
            }
            let image = collect.image;
            let mut fill = |sink: &mut dyn RowSink| -> io::Result<()> {
                sink.start(image.width, image.height, image.color)?;
                for y in 0..image.height {
                    sink.row(image.row(y))?;
                }
                Ok(())
            };
            match self.document.pixel_image(&mut fill) {
                Ok(object) => object,
                Err(error) => {
                    self.error = Some(error);
                    return false;
                }
            }
        };
        let Some(object) = object else {
            return false;
        };
        self.flush_runs();
        let (mut width, mut height) = size.unwrap_or_else(|| natural_size(&object));
        let room_width = self.right() - self.left();
        let room_height = self.setup.height - 2.0 * self.setup.margin;
        let scale = (room_width / width).min(room_height / height).min(1.0);
        if scale.is_finite() && scale > 0.0 {
            width *= scale;
            height *= scale;
        }
        self.room(height);
        self.page_images.push(object.number);
        self.content.push_str(&format!(
            "q {} 0 0 {} {} {} cm /I{} Do Q\n",
            number_text(width),
            number_text(height),
            number_text(self.left()),
            number_text(self.top - height),
            self.page_images.len()
        ));
        self.top -= height;
        self.page_has_content = true;
        self.pending_space = BLOCK_SPACE;
        self.notes.images_placed += 1;
        true
    }

    fn use_font(&mut self, font: StandardFont) {
        if !self.fonts.contains(&font) {
            self.fonts.push(font);
        }
    }

    /// Sets one line whose top is `self.top`, `leading` tall, at `left`.
    fn set_line(&mut self, line: &Line, left: f64, size: f64, leading: f64) {
        let baseline = self.top - leading * 0.78;
        for piece in line {
            if piece.text.is_empty() {
                continue;
            }
            let colored = piece.style.link.is_some();
            if colored {
                self.content.push_str(LINK_COLOR);
                self.content.push('\n');
            }
            // Runs of characters set from one face.
            let mut x = left + piece.x;
            let chars: Vec<char> = piece.text.chars().collect();
            let mut start = 0;
            while start < chars.len() {
                let face = self.faces.face(chars[start], piece.style.code);
                let mut end = start + 1;
                while end < chars.len() && self.faces.face(chars[end], piece.style.code) == face {
                    end += 1;
                }
                let segment: String = chars[start..end].iter().collect();
                let width = self.faces.width(&piece.style, size, &segment);
                self.set_segment(face, &piece.style, &segment, x, baseline, size);
                x += width;
                start = end;
            }
            if colored {
                self.content.push_str("0 g\n");
            }
            if let Some(target) = &piece.style.link {
                let x = left + piece.x;
                self.links.push((
                    [
                        x,
                        baseline - size * 0.25,
                        x + piece.width,
                        baseline + size * 0.8,
                    ],
                    target.clone(),
                ));
            }
        }
        self.top -= leading;
        self.page_has_content = true;
    }

    /// Shows text of one face at a position.
    fn set_segment(
        &mut self,
        face: Face,
        style: &Style,
        text: &str,
        x: f64,
        baseline: f64,
        size: f64,
    ) {
        let (resource, string) = match face {
            Face::Embedded(slot) => {
                let index = match self.faces.embedded[slot] {
                    Some(index) => index,
                    None => {
                        let index = self.document.embed(self.faces.fonts[slot].clone());
                        self.faces.embedded[slot] = Some(index);
                        index
                    }
                };
                if !self.page_embedded.contains(&index) {
                    self.page_embedded.push(index);
                }
                let mut hex = String::from("<");
                for char in text.chars() {
                    let glyph = self.faces.fonts[slot].glyph(char).unwrap_or(0);
                    self.document.use_glyph(index, glyph, char);
                    hex.push_str(&format!("{glyph:04X}"));
                }
                hex.push('>');
                (format!("E{}", index + 1), hex)
            }
            Face::Standard | Face::Missing => {
                let font = style.font();
                self.use_font(font);
                let (bytes, missing) = win_ansi(text);
                self.notes.unset_characters += missing;
                (font.resource().to_string(), literal(&bytes))
            }
        };
        self.content.push_str(&format!(
            "BT /{resource} {} Tf 1 0 0 1 {} {} Tm {string} Tj ET\n",
            number_text(size),
            number_text(x),
            number_text(baseline),
        ));
    }

    // ----------------------------------------------------------- blocks

    /// Sets the collected runs as a paragraph (or heading, or item).
    fn flush_runs(&mut self) {
        if self.runs.is_empty() {
            return;
        }
        let runs = std::mem::take(&mut self.runs);
        let (size, leading) = match self.heading {
            Some(level) => {
                let size = HEADING_SIZES[usize::from(level.clamp(1, 6)) - 1];
                (size, size * 1.3)
            }
            None => (BODY, BODY_LEADING),
        };
        let left = self.left();
        let width = self.right() - left;
        let lines = break_lines(&runs, width, size, &mut self.faces);
        if lines.is_empty() {
            return;
        }
        // A heading keeps the heading and two lines after it together.
        let first_room = if self.heading.is_some() {
            leading * lines.len() as f64 + 2.0 * BODY_LEADING
        } else {
            leading
        };
        if self.heading.is_some() && self.page_has_content {
            self.pending_space = self.pending_space.max(BLOCK_SPACE + 6.0);
        }
        self.room(first_room);
        let marker = self.marker.take();
        for (index, line) in lines.iter().enumerate() {
            if index > 0 {
                self.room(leading);
            }
            if index == 0
                && let Some(marker) = &marker
            {
                let marker_width = self.faces.width(&Style::default(), size, marker);
                let x = left - marker_width - 5.0;
                let piece = Piece {
                    text: marker.clone(),
                    style: Style::default(),
                    x: 0.0,
                    width: marker_width,
                };
                let saved = self.top;
                self.set_line(&vec![piece], x, size, leading);
                self.top = saved;
            }
            if self.quotes > 0 {
                self.quote_bars(leading);
            }
            self.set_line(line, left, size, leading);
        }
        let tight_item = self.lists.last().is_some_and(|list| list.tight);
        self.pending_space = if self.heading.is_some() {
            4.0
        } else if tight_item {
            2.0
        } else {
            BLOCK_SPACE
        };
        if self.heading.is_some() && !self.title_taken {
            self.title_taken = true;
            let title = std::mem::take(&mut self.heading_text);
            self.document.set_title(title.trim());
        }
    }

    /// The bars at the left of a quoted line.
    fn quote_bars(&mut self, height: f64) {
        for depth in 0..self.quotes {
            let x = self.setup.margin
                + self.lists.len() as f64 * LIST_INDENT
                + depth as f64 * QUOTE_INDENT
                + 3.0;
            self.content.push_str(&format!(
                "0.75 g {} {} 2 {} re f 0 g\n",
                number_text(x),
                number_text(self.top - height),
                number_text(height)
            ));
        }
    }

    fn flush_code(&mut self, text: &str) {
        let left = self.left();
        let width = self.right() - left;
        let inner = width - 12.0;
        let character = CODE_SIZE * 0.6;
        let per_line = ((inner / character).floor() as usize).max(1);
        let mut lines: Vec<String> = Vec::new();
        for line in text.trim_end_matches('\n').split('\n') {
            let line = line.replace('\t', "    ");
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                lines.push(String::new());
            }
            for chunk in chars.chunks(per_line) {
                lines.push(chunk.iter().collect());
            }
        }
        let style = Style {
            code: true,
            ..Style::default()
        };
        self.room(CODE_LEADING + 8.0);
        self.band(left, width, 4.0);
        for line in &lines {
            self.room(CODE_LEADING);
            self.band(left, width, CODE_LEADING);
            let piece = Piece {
                text: line.clone(),
                style: style.clone(),
                x: 6.0,
                width: 0.0,
            };
            self.set_line(&vec![piece], left, CODE_SIZE, CODE_LEADING);
        }
        self.band(left, width, 4.0);
        self.top -= 4.0;
        self.pending_space = BLOCK_SPACE;
    }

    /// A light gray band `height` tall below the current top.
    fn band(&mut self, left: f64, width: f64, height: f64) {
        self.content.push_str(&format!(
            "0.95 g {} {} {} {} re f 0 g\n",
            number_text(left),
            number_text(self.top - height),
            number_text(width),
            number_text(height)
        ));
        self.page_has_content = true;
    }

    fn rule(&mut self) {
        self.room(BLOCK_SPACE * 2.0);
        let y = self.top - BLOCK_SPACE;
        self.content.push_str(&format!(
            "0.6 G 0.75 w {} {} m {} {} l S 0 G\n",
            number_text(self.left()),
            number_text(y),
            number_text(self.right()),
            number_text(y)
        ));
        self.top -= BLOCK_SPACE * 2.0;
        self.page_has_content = true;
        self.pending_space = 0.0;
    }

    fn flush_table(&mut self, table: TableBuild) {
        let columns = table
            .rows
            .iter()
            .map(|(_, cells)| cells.len())
            .max()
            .unwrap_or(0);
        if columns == 0 {
            return;
        }
        let left = self.left();
        let available = self.right() - left;
        // Natural width: the longest line unbroken; least: the longest word.
        let mut natural = vec![0.0f64; columns];
        let mut least = vec![0.0f64; columns];
        for (header, cells) in &table.rows {
            for (column, cell) in cells.iter().enumerate() {
                let bold = *header;
                let mut line = 0.0;
                for run in cell {
                    let mut style = run.style.clone();
                    style.bold |= bold;
                    line += self.faces.width(&style, BODY, &run.text);
                    for word in run.text.split_whitespace() {
                        least[column] = least[column].max(self.faces.width(&style, BODY, word));
                    }
                }
                natural[column] = natural[column].max(line);
            }
        }
        let pad = 2.0 * CELL_PAD;
        let natural_total: f64 = natural.iter().map(|width| width + pad).sum();
        let widths: Vec<f64> = if natural_total <= available {
            natural.iter().map(|width| width + pad).collect()
        } else {
            let least_total: f64 = least.iter().map(|width| width + pad).sum();
            let extra = (available - least_total).max(0.0);
            let wants: f64 = natural
                .iter()
                .zip(&least)
                .map(|(n, l)| n - l)
                .sum::<f64>()
                .max(1.0);
            natural
                .iter()
                .zip(&least)
                .map(|(n, l)| l + pad + extra * (n - l) / wants)
                .collect()
        };
        self.room(BODY_LEADING + pad);
        for (header, cells) in &table.rows {
            let mut cell_lines: Vec<Vec<Line>> = Vec::new();
            for (column, width) in widths.iter().enumerate() {
                let mut runs: Vec<Run> = cells.get(column).cloned().unwrap_or_default();
                if *header {
                    for run in &mut runs {
                        run.style.bold = true;
                    }
                }
                cell_lines.push(break_lines(&runs, width - pad, BODY, &mut self.faces));
            }
            let height = cell_lines.iter().map(Vec::len).max().unwrap_or(1).max(1) as f64
                * BODY_LEADING
                + pad;
            if self.page_has_content && self.top - height < self.setup.margin {
                self.end_page();
            }
            let row_top = self.top;
            let mut x = left;
            for (column, lines) in cell_lines.iter().enumerate() {
                let width = widths[column];
                let alignment = table
                    .alignments
                    .get(column)
                    .copied()
                    .unwrap_or(Alignment::None);
                self.top = row_top - CELL_PAD;
                for line in lines {
                    let used = line.last().map_or(0.0, |piece| piece.x + piece.width);
                    let shift = match alignment {
                        Alignment::Right => width - pad - used,
                        Alignment::Center => (width - pad - used) / 2.0,
                        _ => 0.0,
                    };
                    self.set_line(line, x + CELL_PAD + shift.max(0.0), BODY, BODY_LEADING);
                }
                x += width;
            }
            // The row's grid.
            let bottom = row_top - height;
            let right = left + widths.iter().sum::<f64>();
            let mut grid = format!(
                "0.5 G 0.5 w {l} {t} m {r} {t} l S {l} {b} m {r} {b} l S",
                l = number_text(left),
                r = number_text(right),
                t = number_text(row_top),
                b = number_text(bottom)
            );
            let mut edge = left;
            for width in std::iter::once(&0.0).chain(widths.iter()) {
                edge += width;
                grid.push_str(&format!(
                    " {x} {t} m {x} {b} l S",
                    x = number_text(edge),
                    t = number_text(row_top),
                    b = number_text(bottom)
                ));
            }
            grid.push_str(" 0 G\n");
            self.content.push_str(&grid);
            self.top = bottom;
            self.page_has_content = true;
        }
        self.pending_space = BLOCK_SPACE;
    }
}

impl<'a> EventSink<'a> for Composer<'_, '_> {
    fn event(&mut self, event: Event<'a>) {
        self.transient(event);
    }

    fn transient(&mut self, event: Event<'_>) {
        if self.error.is_some() {
            return;
        }
        match event {
            Event::Start(tag) => match tag {
                Tag::Paragraph => self.flush_runs(),
                Tag::Heading(level) => {
                    self.flush_runs();
                    self.heading = Some(level);
                }
                Tag::BlockQuote => {
                    self.flush_runs();
                    self.quotes += 1;
                }
                Tag::CodeBlock(_) => {
                    self.flush_runs();
                    self.code = Some(String::new());
                }
                Tag::List { start, tight } => {
                    self.flush_runs();
                    self.lists.push(List { next: start, tight });
                }
                Tag::Item => {
                    self.flush_runs();
                    let marker = match self.lists.last_mut() {
                        Some(List {
                            next: Some(number), ..
                        }) => {
                            let marker = format!("{number}.");
                            *number += 1;
                            marker
                        }
                        _ => "\u{2022}".to_string(),
                    };
                    self.marker = Some(marker);
                }
                Tag::FootnoteDefinition(label) => {
                    self.flush_runs();
                    self.marker = Some(format!("{label}."));
                }
                Tag::Table(alignments) => {
                    self.flush_runs();
                    self.table = Some(TableBuild {
                        alignments,
                        ..TableBuild::default()
                    });
                }
                Tag::TableHead => {
                    if let Some(table) = &mut self.table {
                        table.in_head = true;
                        table.rows.push((true, Vec::new()));
                    }
                }
                Tag::TableRow => {
                    if let Some(table) = &mut self.table {
                        let header = table.in_head;
                        table.rows.push((header, Vec::new()));
                    }
                }
                Tag::TableCell => {
                    if let Some((_, cells)) =
                        self.table.as_mut().and_then(|table| table.rows.last_mut())
                    {
                        cells.push(Vec::new());
                    }
                }
                Tag::Emphasis => self.italic += 1,
                Tag::Strong => self.bold += 1,
                Tag::Strikethrough => {}
                Tag::Link { destination, .. } => self.links_open.push(destination.into_owned()),
                Tag::Image { destination, .. } => {
                    self.image_depth += 1;
                    // A picture is set as a block of its own; in a table or
                    // inside another image it is its alt text.
                    if self.placed_depth > 0
                        || (self.table.is_none() && self.place_image(&destination))
                    {
                        self.placed_depth += 1;
                    } else {
                        self.notes.images += 1;
                        self.push_text("[", false);
                    }
                }
                Tag::HtmlBlock => {}
            },
            Event::End(tag) => match tag {
                TagEnd::Paragraph => self.flush_runs(),
                TagEnd::Heading(_) => {
                    self.flush_runs();
                    self.heading = None;
                }
                TagEnd::BlockQuote => {
                    self.flush_runs();
                    self.quotes = self.quotes.saturating_sub(1);
                }
                TagEnd::CodeBlock => {
                    if let Some(code) = self.code.take() {
                        self.flush_code(&code);
                    }
                }
                TagEnd::List(_) => {
                    self.flush_runs();
                    self.lists.pop();
                    // Inside a tight list, a sub-list ends as an item does.
                    let tight_parent = self.lists.last().is_some_and(|list| list.tight);
                    self.pending_space = if tight_parent { 2.0 } else { BLOCK_SPACE };
                }
                TagEnd::Item | TagEnd::FootnoteDefinition => {
                    self.flush_runs();
                    self.marker = None;
                }
                TagEnd::Table => {
                    if let Some(table) = self.table.take() {
                        self.flush_table(table);
                    }
                }
                TagEnd::TableHead => {
                    if let Some(table) = &mut self.table {
                        table.in_head = false;
                    }
                }
                TagEnd::TableRow | TagEnd::TableCell => {}
                TagEnd::Emphasis => self.italic = self.italic.saturating_sub(1),
                TagEnd::Strong => self.bold = self.bold.saturating_sub(1),
                TagEnd::Strikethrough | TagEnd::HtmlBlock => {}
                TagEnd::Link => {
                    self.links_open.pop();
                }
                TagEnd::Image => {
                    if self.placed_depth > 0 {
                        self.placed_depth -= 1;
                    } else {
                        self.push_text("]", false);
                    }
                    self.image_depth = self.image_depth.saturating_sub(1);
                }
            },
            Event::Text(text) => match &mut self.code {
                Some(code) => code.push_str(&text),
                // A tab in running text is spacing (tab stops are not
                // laid out); no font draws it.
                None if text.contains('\t') => self.push_text(&text.replace('\t', " "), false),
                None => self.push_text(&text, false),
            },
            Event::Code(text) => self.push_text(&text, true),
            Event::Html(_) | Event::InlineHtml(_) => self.notes.html += 1,
            Event::SoftBreak => self.push_text(" ", false),
            Event::HardBreak => self.push_text("\n", false),
            Event::Rule => {
                self.flush_runs();
                self.rule();
            }
            Event::FootnoteReference(label) => self.push_text(&format!("[{label}]"), false),
            Event::TaskListMarker(done) => {
                self.push_text(if done { "[x] " } else { "[ ] " }, false)
            }
        }
    }
}

// ------------------------------------------------------------- measuring

fn font_widths(font: StandardFont) -> Option<&'static Widths> {
    match font {
        StandardFont::Helvetica | StandardFont::HelveticaOblique => Some(&tables::HELVETICA),
        StandardFont::HelveticaBold | StandardFont::HelveticaBoldOblique => {
            Some(&tables::HELVETICA_BOLD)
        }
        StandardFont::Courier | StandardFont::CourierBold => None,
    }
}

/// The width of a character set in a standard `font` at `size`.
fn char_width(font: StandardFont, size: f64, char: char) -> f64 {
    let Some(widths) = font_widths(font) else {
        return 0.6 * size;
    };
    let code = u16::try_from(u32::from(char)).unwrap_or(u16::MAX);
    let width = widths
        .pairs
        .binary_search_by_key(&code, |pair| pair.0)
        .map_or(widths.default, |index| widths.pairs[index].1);
    f64::from(width) / 1000.0 * size
}

/// A character's WinAnsi byte, when it has one.
fn win_ansi_byte(char: char) -> Option<u8> {
    if (' '..='~').contains(&char) {
        return Some(char as u8);
    }
    let code = u16::try_from(u32::from(char)).ok()?;
    let reverse = win_ansi_reverse();
    reverse
        .binary_search_by_key(&code, |pair| pair.0)
        .ok()
        .map(|index| reverse[index].1)
}

fn win_ansi_reverse() -> &'static [(u16, u8)] {
    static REVERSE: OnceLock<Vec<(u16, u8)>> = OnceLock::new();
    REVERSE.get_or_init(|| {
        let mut pairs: Vec<(u16, u8)> = tables::WIN_ANSI
            .iter()
            .enumerate()
            .filter(|(code, value)| **value >= 0x20 && *code >= 0x20)
            .map(|(code, value)| (*value, code as u8))
            .collect();
        pairs.sort_unstable();
        pairs.dedup_by_key(|pair| pair.0);
        pairs
    })
}

/// `text` as WinAnsi bytes, and how many characters WinAnsi lacks (each
/// set as `?`).
fn win_ansi(text: &str) -> (Vec<u8>, usize) {
    let mut bytes = Vec::with_capacity(text.len());
    let mut missing = 0;
    for char in text.chars() {
        match win_ansi_byte(char) {
            Some(byte) => bytes.push(byte),
            None => {
                bytes.push(b'?');
                missing += 1;
            }
        }
    }
    (bytes, missing)
}

/// Breaks runs into lines at most `width` wide, first fit: words go on a
/// line while they fit; a word wider than a line is split by characters;
/// a `"\n"` run ends the line.
fn break_lines(runs: &[Run], width: f64, size: f64, faces: &mut Faces) -> Vec<Line> {
    // Words: pieces of one or more styles with no space between them,
    // each with the space that follows it.
    struct Word {
        pieces: Vec<(String, Style)>,
        space_after: Option<Style>,
        hard_break: bool,
    }
    let mut words: Vec<Word> = Vec::new();
    let mut current = Word {
        pieces: Vec::new(),
        space_after: None,
        hard_break: false,
    };
    for run in runs {
        if run.text == "\n" {
            current.hard_break = true;
            words.push(std::mem::replace(
                &mut current,
                Word {
                    pieces: Vec::new(),
                    space_after: None,
                    hard_break: false,
                },
            ));
            continue;
        }
        for (index, part) in run.text.split(' ').enumerate() {
            if index > 0 {
                current.space_after = Some(run.style.clone());
                words.push(std::mem::replace(
                    &mut current,
                    Word {
                        pieces: Vec::new(),
                        space_after: None,
                        hard_break: false,
                    },
                ));
            }
            if !part.is_empty() {
                current.pieces.push((part.to_string(), run.style.clone()));
            }
        }
    }
    words.push(current);
    let mut lines: Vec<Line> = Vec::new();
    let mut line: Line = Vec::new();
    let mut x = 0.0;
    let mut space: Option<(f64, Style)> = None;
    for word in words {
        let word_width: f64 = word
            .pieces
            .iter()
            .map(|(text, style)| faces.width(style, size, text))
            .sum();
        if !word.pieces.is_empty() {
            let gap = space.as_ref().map_or(0.0, |(width, _)| *width);
            if !line.is_empty() && x + gap + word_width > width {
                lines.push(std::mem::take(&mut line));
                x = 0.0;
            } else if !line.is_empty() {
                if let Some((space_width, style)) = &space {
                    line.push(Piece {
                        text: " ".to_string(),
                        style: style.clone(),
                        x,
                        width: *space_width,
                    });
                    x += space_width;
                }
            }
            for (text, style) in word.pieces {
                let piece_width = faces.width(&style, size, &text);
                if x + piece_width > width && x == 0.0 {
                    // Wider than a line on its own: split by characters.
                    let mut chunk = String::new();
                    let mut chunk_width = 0.0;
                    for char in text.chars() {
                        let char_width = faces.width(&style, size, char.encode_utf8(&mut [0; 4]));
                        if chunk_width + char_width > width && !chunk.is_empty() {
                            line.push(Piece {
                                text: std::mem::take(&mut chunk),
                                style: style.clone(),
                                x: 0.0,
                                width: chunk_width,
                            });
                            lines.push(std::mem::take(&mut line));
                            chunk_width = 0.0;
                        }
                        chunk.push(char);
                        chunk_width += char_width;
                    }
                    line.push(Piece {
                        text: chunk,
                        style: style.clone(),
                        x: 0.0,
                        width: chunk_width,
                    });
                    x = chunk_width;
                    continue;
                }
                line.push(Piece {
                    text,
                    style,
                    x,
                    width: piece_width,
                });
                x += piece_width;
            }
        }
        space = word
            .space_after
            .map(|style| (faces.width(&style, size, " "), style));
        if word.hard_break {
            lines.push(std::mem::take(&mut line));
            x = 0.0;
            space = None;
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(text: &str) -> Run {
        Run {
            text: text.to_string(),
            style: Style::default(),
        }
    }

    #[test]
    fn words_fill_lines_first_fit() {
        // "aaaa " is 4 x 556 + 278 = 2502 units: two words fit in 5 points
        // at size 1, three do not.
        let lines = break_lines(&[run("aaaa aaaa aaaa")], 5.0, 1.0, &mut Faces::standard());
        let texts: Vec<String> = lines
            .iter()
            .map(|line| line.iter().map(|piece| piece.text.as_str()).collect())
            .collect();
        assert_eq!(texts, ["aaaa aaaa", "aaaa"]);
    }

    #[test]
    fn win_ansi_maps_and_counts_the_rest() {
        let (bytes, missing) = win_ansi("a€“é—✓");
        assert_eq!(bytes, [b'a', 0x80, 0x93, 0xe9, 0x97, b'?']);
        assert_eq!(missing, 1);
    }
}
