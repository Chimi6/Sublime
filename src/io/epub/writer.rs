//! Writing the document model as an EPUB 3 book (with an EPUB 2 table of
//! contents for older readers). The body splits into chapters at page
//! breaks, new-page sections, and its top headings; each chapter is an
//! XHTML file whose notes are EPUB footnotes at its end, which readers
//! show as pop-ups. Named paragraph styles become CSS classes (their
//! sizes and spacing in `em` of the body text, so a reader's font size
//! still scales them), direct formatting becomes inline style, and the
//! document's pictures and embedded fonts go into the package. Pages,
//! headers and footers, comments, and deleted text have no place in a
//! reflowing book and are left out.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;

use super::is_external;
use crate::document::from_events::{StyleKind, is_code_character_style, paragraph_kind_of};
use crate::document::{
    Alignment, Baseline, Block, Caps, Color, Document, FloatingContent, Inline, LineSpacing,
    ListLabel, Merge, NoteId, NumberKind, Paragraph, ParagraphProperties, Run, RunProperties,
    SectionStart, Table, VerticalAlignment,
};
use crate::format::Effort;
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

const MIMETYPE: &[u8] = b"application/epub+zip";

const CONTAINER: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<container version=\"1.0\" xmlns=\"urn:oasis:names:tc:opendocument:xmlns:container\">",
    "<rootfiles><rootfile full-path=\"EPUB/content.opf\" media-type=\"application/oebps-package+xml\"/></rootfiles>",
    "</container>\n"
);

/// What every book's style sheet starts with: notes and tables readable,
/// pictures inside the page, and code kept as written.
const BASE_CSS: &str = "\
img { max-width: 100%; }
table { border-collapse: collapse; margin: 0.5em 0; }
td, th { vertical-align: top; padding: 0.1em 0.4em; text-align: left; }
table.grid td, table.grid th { border: 1px solid; }
pre { white-space: pre-wrap; }
a.noteref { vertical-align: super; font-size: 0.75em; line-height: 0; text-decoration: none; }
section.notes { margin-top: 2em; font-size: 0.9em; }
aside.footnote p { margin: 0.25em 0; }
a.noteback { text-decoration: none; }
.textbox { margin: 0.5em 0; padding: 0.25em 0.5em; border: 1px solid; }
";

/// Writes `document` as an `.epub` to `sink`.
pub fn write_epub<W: io::Write>(document: &Document, sink: W, effort: Effort) -> io::Result<W> {
    let mut zip = ZipWriter::new(sink).with_effort(effort);
    zip.add("mimetype", MIMETYPE)?;
    zip.add_deflated("META-INF/container.xml", CONTAINER.as_bytes())?;
    let mut writer = EpubWriter::new(document);
    writer.body(&mut zip)?;
    let css = writer.css();
    zip.add_deflated("EPUB/style.css", css.as_bytes())?;
    for (path, media) in &writer.media_paths {
        zip.add(&format!("EPUB/{path}"), &document.media[*media].bytes)?;
    }
    for (index, font) in document.fonts.iter().enumerate() {
        zip.add_deflated(&format!("EPUB/{}", font_path(index, font)), &font.bytes)?;
    }
    zip.add_deflated("EPUB/nav.xhtml", writer.nav().as_bytes())?;
    zip.add_deflated("EPUB/toc.ncx", writer.ncx().as_bytes())?;
    zip.add_deflated("EPUB/content.opf", writer.opf().as_bytes())?;
    zip.finish()
}

/// A finished chapter: its file and title.
struct Chapter {
    file: String,
    title: String,
    math: bool,
}

/// A heading for the tables of contents.
struct Heading {
    chapter: usize,
    level: u8,
    id: String,
    text: String,
}

/// An open list: whether it is numbered, and whether an item is open in it.
struct OpenList {
    ordered: bool,
    item_open: bool,
}

struct EpubWriter<'d> {
    document: &'d Document,
    /// The body text's size in points, which `em` measures from.
    base_size: f32,
    /// The text column's width in points, which pictures are a share of.
    text_width: f32,
    /// Each paragraph style's class name.
    classes: Vec<String>,
    /// Whether each paragraph style is used, so only those get rules.
    used: Vec<bool>,
    /// Each style's kind (plain, code, quote, rule), by its name.
    kinds: Vec<StyleKind>,
    /// Character styles that mark code.
    code_styles: Vec<bool>,
    /// The heading level a style stands for, when it does.
    style_levels: Vec<Option<u8>>,
    /// The level the body splits at: headings this high or higher start a
    /// chapter.
    split_level: u8,
    chapters: Vec<Chapter>,
    headings: Vec<Heading>,
    /// The chapter being written: its body and the notes it refers to.
    current: String,
    current_notes: Vec<NoteId>,
    current_math: bool,
    /// Whether the current chapter has any text yet.
    has_content: bool,
    /// The paragraph before ended in a page break.
    break_pending: bool,
    lists: Vec<OpenList>,
    /// Each note's number, given at its first reference.
    note_numbers: Vec<Option<u32>>,
    next_note: u32,
    /// Pictures written, by media id, and their package paths.
    media_paths: Vec<(String, usize)>,
    media_index: HashMap<usize, Option<String>>,
    /// How deep in table cells the writer is.
    cell_depth: usize,
}

impl<'d> EpubWriter<'d> {
    fn new(document: &'d Document) -> Self {
        let styles = &document.styles;
        let mut taken: Vec<String> = Vec::new();
        let classes = styles
            .paragraph
            .iter()
            .map(|style| {
                let mut class: String = style
                    .name
                    .chars()
                    .map(|ch| {
                        if ch.is_ascii_alphanumeric() {
                            ch.to_ascii_lowercase()
                        } else {
                            '-'
                        }
                    })
                    .collect();
                class = class.trim_matches('-').to_string();
                if class.is_empty() {
                    class = "style".to_string();
                }
                let mut unique = format!("s-{class}");
                let mut number = 2;
                while taken.contains(&unique) {
                    unique = format!("s-{class}-{number}");
                    number += 1;
                }
                taken.push(unique.clone());
                unique
            })
            .collect();
        let style_levels: Vec<Option<u8>> = (0..styles.paragraph.len())
            .map(|id| {
                if styles.paragraph[id].name == "Title" {
                    return Some(0);
                }
                document.paragraph_style_properties(id).outline_level
            })
            .collect();
        let base_size = styles
            .default_paragraph
            .and_then(|style| document.paragraph_style_run(style).size)
            .unwrap_or(12.0)
            .max(1.0);
        let text_width = document
            .sections
            .first()
            .map(|section| {
                section.page.width - section.page.margin_left - section.page.margin_right
            })
            .filter(|width| *width > 0.0)
            .unwrap_or(468.0);
        let mut writer = EpubWriter {
            document,
            base_size,
            text_width,
            classes,
            used: vec![false; styles.paragraph.len()],
            kinds: styles
                .paragraph
                .iter()
                .map(|style| paragraph_kind_of(&style.name))
                .collect(),
            code_styles: styles
                .character
                .iter()
                .map(|style| is_code_character_style(&style.name))
                .collect(),
            style_levels,
            split_level: 0,
            chapters: Vec::new(),
            headings: Vec::new(),
            current: String::new(),
            current_notes: Vec::new(),
            current_math: false,
            has_content: false,
            break_pending: false,
            lists: Vec::new(),
            note_numbers: vec![None; document.footnotes.len()],
            next_note: 1,
            media_paths: Vec::new(),
            media_index: HashMap::new(),
            cell_depth: 0,
        };
        writer.split_level = writer.choose_split_level();
        writer
    }

    /// The highest heading level that comes more than once: a book's
    /// chapters. A title alone above them does not count.
    fn choose_split_level(&self) -> u8 {
        let mut counts = [0usize; 9];
        for section in &self.document.sections {
            for block in &section.blocks {
                if let Block::Paragraph(paragraph) = block
                    && let Some(level) = self.heading_level(paragraph)
                {
                    counts[usize::from(level.min(8))] += 1;
                }
            }
        }
        // Level 0 is a heading 1 (or a title).
        counts
            .iter()
            .position(|count| *count >= 2)
            .map_or(0, |level| level as u8)
    }

    /// A paragraph's heading level (0 for heading 1), when it is one.
    fn heading_level(&self, paragraph: &Paragraph) -> Option<u8> {
        if let Some(level) = self.document.paragraph_properties(paragraph).outline_level {
            return Some(level);
        }
        let style = paragraph.style.or(self.document.styles.default_paragraph)?;
        self.style_levels.get(style).copied().flatten()
    }

    fn kind(&self, paragraph: &Paragraph) -> StyleKind {
        paragraph
            .style
            .and_then(|style| self.kinds.get(style).copied())
            .unwrap_or(StyleKind::Plain)
    }

    // ----- chapters -----

    fn body<W: io::Write>(&mut self, zip: &mut ZipWriter<W>) -> io::Result<()> {
        let document = self.document;
        for (index, section) in document.sections.iter().enumerate() {
            if index > 0 && section.start == SectionStart::NewPage {
                self.break_pending = true;
            }
            for block in &section.blocks {
                if let Block::Paragraph(paragraph) = block {
                    let page_break = self.break_pending
                        || document.effective_paragraph(paragraph).page_break_before == Some(true)
                        || matches!(
                            paragraph.runs.first(),
                            Some(Run {
                                content: Inline::PageBreak,
                                ..
                            })
                        );
                    let heading = self
                        .heading_level(paragraph)
                        .is_some_and(|level| level <= self.split_level);
                    if (page_break || heading) && self.has_content {
                        self.finish_chapter(zip)?;
                    }
                    self.break_pending = matches!(
                        paragraph.runs.last(),
                        Some(Run {
                            content: Inline::PageBreak,
                            ..
                        })
                    ) && paragraph.runs.len() > 1;
                    if paragraph
                        .runs
                        .iter()
                        .all(|run| matches!(run.content, Inline::PageBreak | Inline::ColumnBreak))
                        && !paragraph.runs.is_empty()
                    {
                        // The break itself: the chapter split is its trace.
                        continue;
                    }
                }
                let mut out = std::mem::take(&mut self.current);
                self.block(block, &mut out);
                self.current = out;
                self.has_content = true;
            }
        }
        // Objects placed on the page rather than in the text come last.
        let mut out = std::mem::take(&mut self.current);
        self.close_lists(&mut out);
        for (index, object) in document.floating.iter().enumerate() {
            if !object.follows_text {
                self.floating(index, &mut out);
            }
        }
        self.current = out;
        if self.has_content || self.chapters.is_empty() {
            self.finish_chapter(zip)?;
        }
        Ok(())
    }

    fn finish_chapter<W: io::Write>(&mut self, zip: &mut ZipWriter<W>) -> io::Result<()> {
        let mut body = std::mem::take(&mut self.current);
        self.close_lists(&mut body);
        let number = self.chapters.len() + 1;
        let title = self
            .headings
            .iter()
            .find(|heading| heading.chapter == self.chapters.len() && !heading.text.is_empty())
            .map_or_else(|| format!("Part {number}"), |heading| heading.text.clone());
        // The chapter's notes, each with the notes its notes refer to.
        let mut notes = String::new();
        let mut index = 0;
        while index < self.current_notes.len() {
            let note = self.current_notes[index];
            index += 1;
            let number = self.note_numbers[note].unwrap_or(0);
            let _ = write!(
                notes,
                "<aside id=\"fn-{number}\" class=\"footnote\" epub:type=\"footnote\" role=\"doc-footnote\">"
            );
            let start = notes.len();
            let blocks = &self.document.footnotes[note].blocks;
            for block in blocks {
                self.block(block, &mut notes);
            }
            self.close_lists(&mut notes);
            // The note's number leads its first paragraph, as a way back
            // to its reference (which readers leave out of its text).
            let label = format!(
                "<a class=\"noteback\" href=\"#fnref-{number}\" role=\"doc-backlink\">{number}.</a> "
            );
            match notes[start..].find("<p").and_then(|at| {
                notes[start + at..]
                    .find('>')
                    .map(|end| start + at + end + 1)
            }) {
                Some(at) => notes.insert_str(at, &label),
                None => notes.insert_str(start, &format!("<p>{label}</p>")),
            }
            notes.push_str("</aside>");
        }
        let file = format!("ch{number:03}.xhtml");
        let mut xhtml = String::with_capacity(body.len() + notes.len() + 512);
        xhtml.push_str(&page_head(&title, &self.language()));
        xhtml.push_str(&body);
        if !notes.is_empty() {
            xhtml.push_str(
                "<section class=\"notes\" epub:type=\"footnotes\" role=\"doc-endnotes\">",
            );
            xhtml.push_str(&notes);
            xhtml.push_str("</section>");
        }
        xhtml.push_str("</body>\n</html>\n");
        zip.add_deflated(&format!("EPUB/text/{file}"), xhtml.as_bytes())?;
        self.chapters.push(Chapter {
            file,
            title,
            math: self.current_math,
        });
        self.current_notes.clear();
        self.current_math = false;
        self.has_content = false;
        Ok(())
    }

    // ----- blocks -----

    fn block(&mut self, block: &Block, out: &mut String) {
        match block {
            Block::Paragraph(paragraph) => self.paragraph(paragraph, out),
            Block::Table(table) => {
                self.close_lists(out);
                self.table(table, out);
            }
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, out: &mut String) {
        if let Some(item) = paragraph.list {
            self.list_item(paragraph, item, out);
            return;
        }
        self.close_lists(out);
        match self.kind(paragraph) {
            StyleKind::Rule => {
                out.push_str("<hr/>");
                return;
            }
            StyleKind::Code => {
                // Consecutive code lines join in one block.
                let text = self.document.paragraph_text(paragraph);
                if out.ends_with("</code></pre>") {
                    out.truncate(out.len() - "</code></pre>".len());
                    out.push('\n');
                } else {
                    out.push_str("<pre><code>");
                }
                escape_text(out, &clean(&text));
                out.push_str("</code></pre>");
                return;
            }
            StyleKind::Quote => {
                out.push_str("<blockquote>");
                self.paragraph_element(paragraph, out, false);
                out.push_str("</blockquote>");
                return;
            }
            StyleKind::Plain => {}
        }
        self.paragraph_element(paragraph, out, false);
    }

    /// A `<p>` or heading with its style, its own formatting, and its runs.
    fn paragraph_element(&mut self, paragraph: &Paragraph, out: &mut String, in_item: bool) {
        let document = self.document;
        let level = self.heading_level(paragraph);
        let tag = match level {
            Some(level) => format!("h{}", level.min(5) + 1),
            None => "p".to_string(),
        };
        let _ = write!(out, "<{tag}");
        if let Some(level) = level {
            let id = format!("h-{}", self.headings.len() + 1);
            let _ = write!(out, " id=\"{id}\"");
            self.headings.push(Heading {
                chapter: self.chapters.len(),
                level,
                id,
                text: clean(document.paragraph_text(paragraph).trim()),
            });
        }
        let style = paragraph.style.or(document.styles.default_paragraph);
        if let Some(style) = style {
            self.used[style] = true;
            let _ = write!(out, " class=\"{}\"", self.classes[style]);
        }
        let mut own = document.paragraph_properties(paragraph);
        if in_item {
            // The list places its items.
            own.left_indent = None;
            own.first_line_indent = None;
        }
        let mut css = String::new();
        let style_left =
            style.and_then(|style| document.paragraph_style_properties(style).left_indent);
        if in_item
            && style.is_some_and(|style| {
                let properties = document.paragraph_style_properties(style);
                properties.left_indent.is_some() || properties.first_line_indent.is_some()
            })
        {
            // Over the style's indents, which the list already gives.
            css.push_str("margin-left: 0; text-indent: 0; ");
        }
        paragraph_css(&own, style_left, self.base_size, &mut css);
        run_css(
            &document.paragraph_run_properties(paragraph),
            None,
            self.base_size,
            document,
            &mut css,
        );
        if !css.is_empty() {
            out.push_str(" style=\"");
            escape_attribute(out, css.trim_end());
            out.push('"');
        }
        out.push('>');
        let start = out.len();
        let anchors = self.runs(paragraph, level.is_some(), out);
        if out.len() == start && self.cell_depth == 0 {
            // An empty paragraph holds its line, as it does on the page.
            out.push('\u{a0}');
        }
        let _ = write!(out, "</{tag}>");
        for anchor in anchors {
            self.floating(anchor, out);
        }
    }

    fn list_item(
        &mut self,
        paragraph: &Paragraph,
        item: crate::document::ListItem,
        out: &mut String,
    ) {
        let level = usize::from(item.level);
        let label = self
            .document
            .styles
            .list
            .get(item.style)
            .and_then(|style| style.levels.get(level).or(style.levels.last()))
            .map(|level| level.label.clone())
            .unwrap_or(ListLabel::Text("\u{2022}".to_string()));
        let ordered = matches!(label, ListLabel::Number(_));
        // Deeper lists close; this level's item closes, or its list when
        // it restarts or changes kind.
        while self.lists.len() > level + 1 {
            self.close_list(out);
        }
        if self.lists.len() == level + 1 {
            let restart = item.starts_list
                || self
                    .lists
                    .last()
                    .is_some_and(|list| list.ordered != ordered);
            if restart {
                self.close_list(out);
            } else if let Some(list) = self.lists.last_mut()
                && list.item_open
            {
                out.push_str("</li>");
                list.item_open = false;
            }
        }
        while self.lists.len() < level + 1 {
            // A list inside an item; a skipped level gets an unmarked item.
            if let Some(parent) = self.lists.last_mut()
                && !parent.item_open
            {
                out.push_str("<li style=\"list-style-type: none\">");
                parent.item_open = true;
            }
            let deepest = self.lists.len() == level;
            let tag = if ordered && deepest { "ol" } else { "ul" };
            let _ = write!(out, "<{tag}");
            if deepest {
                let mut style = String::new();
                match &label {
                    ListLabel::Number(format) => {
                        if format.kind != NumberKind::Decimal {
                            let kind = match format.kind {
                                NumberKind::LowerLetter => "lower-alpha",
                                NumberKind::UpperLetter => "upper-alpha",
                                NumberKind::LowerRoman => "lower-roman",
                                NumberKind::UpperRoman => "upper-roman",
                                NumberKind::Decimal => "decimal",
                            };
                            let _ = write!(style, "list-style-type: {kind}");
                        }
                        if item.starts_list && item.start != 1 {
                            let _ = write!(out, " start=\"{}\"", item.start);
                        }
                    }
                    ListLabel::Text(marker) => {
                        let kind = match marker.trim() {
                            "\u{2022}" | "\u{25CF}" | "" | "\u{F0B7}" => "disc",
                            "\u{25E6}" | "o" | "\u{25CB}" => "circle",
                            "\u{25AA}" | "\u{25A0}" | "\u{F0A7}" => "square",
                            _ => "disc",
                        };
                        if kind != "disc" {
                            let _ = write!(style, "list-style-type: {kind}");
                        }
                    }
                    ListLabel::None => style.push_str("list-style-type: none"),
                }
                if !style.is_empty() {
                    let _ = write!(out, " style=\"{style}\"");
                }
            }
            out.push('>');
            self.lists.push(OpenList {
                ordered: ordered && deepest,
                item_open: false,
            });
        }
        out.push_str("<li>");
        if let Some(list) = self.lists.last_mut() {
            list.item_open = true;
        }
        self.paragraph_element(paragraph, out, true);
    }

    fn close_list(&mut self, out: &mut String) {
        if let Some(list) = self.lists.pop() {
            if list.item_open {
                out.push_str("</li>");
            }
            out.push_str(if list.ordered { "</ol>" } else { "</ul>" });
        }
    }

    fn close_lists(&mut self, out: &mut String) {
        while !self.lists.is_empty() {
            self.close_list(out);
        }
    }

    fn table(&mut self, table: &Table, out: &mut String) {
        let borders = table.borders.unwrap_or_default();
        let visible = [
            borders.top,
            borders.bottom,
            borders.left,
            borders.right,
            borders.inside_horizontal,
            borders.inside_vertical,
        ]
        .iter()
        .any(|border| border.is_some_and(|border| border.width > 0.0));
        out.push_str(if visible {
            "<table class=\"grid\">"
        } else {
            "<table>"
        });
        let total: f32 = table.columns.iter().sum();
        if total > 0.0 && table.columns.len() > 1 {
            out.push_str("<colgroup>");
            for width in &table.columns {
                let _ = write!(
                    out,
                    "<col style=\"width: {}%\"/>",
                    number(width / total * 100.0)
                );
            }
            out.push_str("</colgroup>");
        }
        let lists = std::mem::take(&mut self.lists);
        for (index, row) in table.rows.iter().enumerate() {
            let head = index < table.header_rows as usize;
            if index == 0 && head {
                out.push_str("<thead>");
            }
            if index == table.header_rows as usize {
                if head || index > 0 {
                    out.push_str("</thead>");
                }
                out.push_str("<tbody>");
            }
            out.push_str("<tr>");
            for cell in &row.cells {
                if cell.merge != Merge::Origin {
                    continue;
                }
                let tag = if head { "th" } else { "td" };
                let _ = write!(out, "<{tag}");
                if cell.column_span > 1 {
                    let _ = write!(out, " colspan=\"{}\"", cell.column_span);
                }
                if cell.row_span > 1 {
                    let _ = write!(out, " rowspan=\"{}\"", cell.row_span);
                }
                let mut css = String::new();
                if let Some(color) = cell.background {
                    let _ = write!(css, "background-color: #{}; ", color.hex().to_lowercase());
                }
                match cell.vertical_alignment {
                    Some(VerticalAlignment::Center) => css.push_str("vertical-align: middle; "),
                    Some(VerticalAlignment::Bottom) => css.push_str("vertical-align: bottom; "),
                    _ => {}
                }
                if !css.is_empty() {
                    let _ = write!(out, " style=\"{}\"", css.trim_end());
                }
                out.push('>');
                self.cell_depth += 1;
                for block in &cell.blocks {
                    self.block(block, out);
                }
                self.cell_depth -= 1;
                self.close_lists(out);
                let _ = write!(out, "</{tag}>");
            }
            out.push_str("</tr>");
        }
        if table.rows.len() <= table.header_rows as usize && table.header_rows > 0 {
            out.push_str("</thead>");
        } else if !table.rows.is_empty() {
            out.push_str("</tbody>");
        }
        out.push_str("</table>");
        self.lists = lists;
    }

    /// A floating object: a picture or a text box as a block of its own.
    fn floating(&mut self, index: usize, out: &mut String) {
        let Some(object) = self.document.floating.get(index) else {
            return;
        };
        match &object.content {
            FloatingContent::Image(media) => {
                out.push_str("<p class=\"picture\">");
                self.image(*media, object.width, None, out);
                out.push_str("</p>");
            }
            FloatingContent::TextBox { blocks, .. } => {
                let lists = std::mem::take(&mut self.lists);
                out.push_str("<div class=\"textbox\">");
                for block in blocks {
                    self.block(block, out);
                }
                self.close_lists(out);
                out.push_str("</div>");
                self.lists = lists;
            }
            _ => {}
        }
    }

    // ----- runs -----

    /// The paragraph's runs; the floating objects anchored in it, to
    /// follow it.
    fn runs(&mut self, paragraph: &Paragraph, heading: bool, out: &mut String) -> Vec<usize> {
        let document = self.document;
        let mut base = match paragraph.style.or(document.styles.default_paragraph) {
            Some(style) => document.paragraph_style_run(style),
            None => RunProperties::default(),
        };
        base.overlay(&document.paragraph_run_properties(paragraph));
        // Bold and italic are said with tags wherever they show, so a
        // reader without the style sheet (a screen reader, another
        // converter) keeps them; a heading's own weight is its heading's.
        let tagged = if heading {
            base
        } else {
            RunProperties::default()
        };
        let mut anchors = Vec::new();
        // The formatting open now (its closing tags) and its link.
        // The elements open now, outermost first, and the link around them.
        let mut stack: Vec<Element> = Vec::new();
        let mut open_link: Option<Option<&str>> = None;
        for run in &paragraph.runs {
            if document.is_deleted(run) {
                continue;
            }
            let properties = document.effective_run(paragraph, run);
            if properties.hidden == Some(true) {
                continue;
            }
            let code = run
                .style
                .is_some_and(|style| self.code_styles.get(style).copied().unwrap_or(false));
            let wanted = run_tags(&properties, &base, &tagged, code, self.base_size, document);
            let link = document.link(run).filter(|target| is_external(target));
            if open_link != Some(link) {
                // A new link closes everything inside the old one.
                close_elements(&mut stack, 0, out);
                if open_link.flatten().is_some() {
                    out.push_str("</a>");
                }
                if let Some(target) = link {
                    out.push_str("<a href=\"");
                    escape_attribute(out, target);
                    out.push_str("\">");
                }
                open_link = Some(link);
            }
            // Elements still wanted stay open (italic carries on into bold
            // italic); the rest close, and new ones open inside.
            let keep = stack
                .iter()
                .take_while(|element| wanted.contains(element))
                .count();
            close_elements(&mut stack, keep, out);
            for element in wanted {
                if !stack.contains(&element) {
                    out.push_str(&element.open);
                    stack.push(element);
                }
            }
            match run.content {
                Inline::Text(span) => escape_text(out, &clean(document.text(span))),
                Inline::LineBreak => out.push_str("<br/>"),
                Inline::Tab => out.push('\u{2003}'),
                Inline::Math(span) => {
                    self.current_math = true;
                    math(document.text(span), out);
                }
                Inline::Footnote(note) => {
                    let number = match self.note_numbers.get(note).copied().flatten() {
                        Some(number) => number,
                        None => {
                            let number = self.next_note;
                            self.next_note += 1;
                            if let Some(slot) = self.note_numbers.get_mut(note) {
                                *slot = Some(number);
                            }
                            self.current_notes.push(note);
                            number
                        }
                    };
                    let _ = write!(
                        out,
                        "<a class=\"noteref\" id=\"fnref-{number}\" href=\"#fn-{number}\" epub:type=\"noteref\" role=\"doc-noteref\">{number}</a>"
                    );
                }
                Inline::Image(id) => {
                    if let Some(image) = document.image(id) {
                        let description = image.description.as_deref();
                        self.image(image.media, image.width, description, out);
                    }
                }
                Inline::Anchor(id) => anchors.push(id as usize),
                Inline::PageBreak
                | Inline::ColumnBreak
                | Inline::PageNumber
                | Inline::PageCount
                | Inline::CommentStart(_)
                | Inline::CommentEnd(_) => {}
            }
        }
        close_elements(&mut stack, 0, out);
        if open_link.flatten().is_some() {
            out.push_str("</a>");
        }
        anchors
    }

    /// A picture, at its share of the text column's width.
    fn image(&mut self, media: usize, width: f32, description: Option<&str>, out: &mut String) {
        let Some(path) = self.media_path(media) else {
            // A picture a book cannot carry (a Windows metafile): its
            // description stands in.
            if let Some(description) = description {
                escape_text(out, description);
            }
            return;
        };
        out.push_str("<img src=\"../");
        escape_attribute(out, &path);
        out.push_str("\" alt=\"");
        escape_attribute(out, description.unwrap_or(""));
        out.push('"');
        if width > 0.0 {
            let share = (width / self.text_width * 100.0).min(100.0);
            let _ = write!(out, " style=\"width: {}%\"", number(share));
        }
        out.push_str("/>");
    }

    /// A picture's path in the package, given the first time it is used;
    /// `None` for a kind of picture EPUB does not carry.
    fn media_path(&mut self, media: usize) -> Option<String> {
        if let Some(path) = self.media_index.get(&media) {
            return path.clone();
        }
        let name = &self.document.media.get(media)?.name;
        let path = media_type(name).map(|_| {
            let clean: String = name
                .rsplit('/')
                .next()
                .unwrap_or(name)
                .chars()
                .map(|ch| {
                    if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                        ch
                    } else {
                        '_'
                    }
                })
                .collect();
            let mut path = format!("media/{clean}");
            let mut number = 2;
            while self.media_paths.iter().any(|(taken, _)| *taken == path) {
                let (stem, extension) = clean.rsplit_once('.').unwrap_or((&clean, ""));
                path = format!("media/{stem}-{number}.{extension}");
                number += 1;
            }
            self.media_paths.push((path.clone(), media));
            path
        });
        self.media_index.insert(media, path.clone());
        path
    }

    // ----- package -----

    /// The book's language: its body style's, or English.
    fn language(&self) -> String {
        let document = self.document;
        document
            .styles
            .default_paragraph
            .and_then(|style| document.paragraph_style_run(style).language)
            .map(|id| document.string(id).replace('_', "-"))
            .filter(|language| {
                // A language tag: letters, digits, and hyphens, starting
                // with a two- or three-letter language.
                let primary = language.split('-').next().unwrap_or("");
                (2..=3).contains(&primary.len())
                    && primary.bytes().all(|byte| byte.is_ascii_alphabetic())
                    && language
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            })
            .unwrap_or_else(|| "en".to_string())
    }

    /// The book's title: its title paragraph, else its first heading.
    fn title(&self) -> String {
        let document = self.document;
        let titled = document
            .sections
            .iter()
            .flat_map(|section| &section.blocks)
            .find_map(|block| {
                let Block::Paragraph(paragraph) = block else {
                    return None;
                };
                let style = paragraph.style?;
                (document.styles.paragraph[style].name == "Title")
                    .then(|| clean(document.paragraph_text(paragraph).trim()))
                    .filter(|text| !text.is_empty())
            });
        titled
            .or_else(|| self.headings.first().map(|heading| heading.text.clone()))
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "Untitled".to_string())
    }

    fn css(&self) -> String {
        let document = self.document;
        let mut css = String::from(BASE_CSS);
        for (index, font) in document.fonts.iter().enumerate() {
            let _ = writeln!(
                css,
                "@font-face {{ font-family: \"{}\"; font-weight: {}; font-style: {}; src: url(\"{}\"); }}",
                font.family.replace('"', ""),
                if font.bold { "bold" } else { "normal" },
                if font.italic { "italic" } else { "normal" },
                font_path(index, font)
            );
        }
        for (style, class) in self.classes.iter().enumerate() {
            if !self.used[style] {
                continue;
            }
            let mut rule = String::new();
            let paragraph = document.paragraph_style_properties(style);
            let stated = paragraph.space_before.is_some() || paragraph.space_after.is_some();
            let mut properties = paragraph;
            if stated {
                properties.space_before.get_or_insert(0.0);
                properties.space_after.get_or_insert(0.0);
            }
            paragraph_css(&properties, None, self.base_size, &mut rule);
            let run = document.paragraph_style_run(style);
            run_css(&run, None, self.base_size, document, &mut rule);
            if !rule.is_empty() {
                let _ = writeln!(css, ".{class} {{ {} }}", rule.trim_end());
            }
        }
        css
    }

    fn nav(&self) -> String {
        let mut out = String::new();
        out.push_str(&page_head("Contents", &self.language()).replace("../style.css", "style.css"));
        out.push_str("<nav epub:type=\"toc\" role=\"doc-toc\" id=\"toc\"><h1>Contents</h1>");
        let entries = self.toc_entries();
        let mut depth = 0usize;
        let mut open_item = false;
        for (level, href, text) in &entries {
            let level = usize::from(*level).min(depth);
            if level + 1 > depth {
                out.push_str("<ol>");
                depth += 1;
                open_item = false;
            }
            while depth > level + 1 {
                out.push_str("</li></ol>");
                depth -= 1;
                open_item = true;
            }
            if open_item {
                out.push_str("</li>");
            }
            out.push_str("<li><a href=\"");
            escape_attribute(&mut out, href);
            out.push_str("\">");
            escape_text(&mut out, text);
            out.push_str("</a>");
            open_item = true;
        }
        while depth > 0 {
            out.push_str("</li></ol>");
            depth -= 1;
        }
        out.push_str("</nav></body>\n</html>\n");
        out
    }

    /// The tables of contents' entries: (depth from 0, link, text), the
    /// headings down to three levels below the chapters' own, or one entry
    /// per chapter in a book without headings.
    fn toc_entries(&self) -> Vec<(u8, String, String)> {
        let top = self
            .headings
            .iter()
            .map(|heading| heading.level)
            .min()
            .unwrap_or(0);
        let entries: Vec<(u8, String, String)> = self
            .headings
            .iter()
            .filter(|heading| heading.level <= top + 2 && !heading.text.is_empty())
            .map(|heading| {
                (
                    heading.level - top,
                    format!(
                        "text/{}#{}",
                        self.chapters[heading.chapter].file, heading.id
                    ),
                    heading.text.clone(),
                )
            })
            .collect();
        if !entries.is_empty() {
            return entries;
        }
        self.chapters
            .iter()
            .map(|chapter| (0, format!("text/{}", chapter.file), chapter.title.clone()))
            .collect()
    }

    fn ncx(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<ncx xmlns=\"http://www.daisy.org/z3986/2005/ncx/\" version=\"2005-1\"><head>",
        );
        let _ = write!(
            out,
            "<meta name=\"dtb:uid\" content=\"{}\"/>",
            self.identifier()
        );
        out.push_str("<meta name=\"dtb:depth\" content=\"3\"/><meta name=\"dtb:totalPageCount\" content=\"0\"/><meta name=\"dtb:maxPageNumber\" content=\"0\"/></head><docTitle><text>");
        escape_text(&mut out, &self.title());
        out.push_str("</text></docTitle><navMap>");
        let mut depth = 0usize;
        for (order, (level, href, text)) in self.toc_entries().iter().enumerate() {
            let level = usize::from(*level).min(depth);
            while depth > level {
                out.push_str("</navPoint>");
                depth -= 1;
            }
            let _ = write!(
                out,
                "<navPoint id=\"nav-{}\" playOrder=\"{}\"><navLabel><text>",
                order + 1,
                order + 1
            );
            escape_text(&mut out, text);
            out.push_str("</text></navLabel><content src=\"");
            escape_attribute(&mut out, href);
            out.push_str("\"/>");
            depth += 1;
        }
        while depth > 0 {
            out.push_str("</navPoint>");
            depth -= 1;
        }
        out.push_str("</navMap></ncx>\n");
        out
    }

    /// A stable identifier from the book's content: the same document
    /// gives the same book.
    fn identifier(&self) -> String {
        let mut hash: u128 = 0x6c62272e07bb014262b821756295c58d;
        for byte in self.document.text.bytes().chain(self.title().bytes()) {
            hash ^= u128::from(byte);
            hash = hash.wrapping_mul(0x0000000001000000000000000000013B);
        }
        let hex = format!("{hash:032x}");
        format!(
            "urn:uuid:{}-{}-4{}-8{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[13..16],
            &hex[17..20],
            &hex[20..32]
        )
    }

    fn opf(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<package xmlns=\"http://www.idpf.org/2007/opf\" version=\"3.0\" unique-identifier=\"uid\" xml:lang=\"",
        );
        escape_attribute(&mut out, &self.language());
        out.push_str(
            "\"><metadata xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:identifier id=\"uid\">",
        );
        out.push_str(&self.identifier());
        out.push_str("</dc:identifier><dc:title>");
        escape_text(&mut out, &self.title());
        out.push_str("</dc:title><dc:language>");
        escape_text(&mut out, &self.language());
        let _ = write!(
            out,
            "</dc:language><meta property=\"dcterms:modified\">{}</meta></metadata><manifest>",
            modified()
        );
        out.push_str("<item id=\"nav\" href=\"nav.xhtml\" media-type=\"application/xhtml+xml\" properties=\"nav\"/>");
        out.push_str("<item id=\"ncx\" href=\"toc.ncx\" media-type=\"application/x-dtbncx+xml\"/>");
        out.push_str("<item id=\"css\" href=\"style.css\" media-type=\"text/css\"/>");
        for (index, chapter) in self.chapters.iter().enumerate() {
            let _ = write!(
                out,
                "<item id=\"ch{}\" href=\"text/{}\" media-type=\"application/xhtml+xml\"{}/>",
                index + 1,
                chapter.file,
                if chapter.math {
                    " properties=\"mathml\""
                } else {
                    ""
                }
            );
        }
        for (index, (path, media)) in self.media_paths.iter().enumerate() {
            let kind = media_type(&self.document.media[*media].name).unwrap_or("image/png");
            let _ = write!(out, "<item id=\"img{}\" href=\"", index + 1);
            escape_attribute(&mut out, path);
            let _ = write!(out, "\" media-type=\"{kind}\"/>");
        }
        for (index, font) in self.document.fonts.iter().enumerate() {
            let kind = if font.bytes.starts_with(b"OTTO") {
                "font/otf"
            } else {
                "font/ttf"
            };
            let _ = write!(out, "<item id=\"font{}\" href=\"", index + 1);
            escape_attribute(&mut out, &font_path(index, font));
            let _ = write!(out, "\" media-type=\"{kind}\"/>");
        }
        out.push_str("</manifest><spine toc=\"ncx\">");
        for index in 0..self.chapters.len() {
            let _ = write!(out, "<itemref idref=\"ch{}\"/>", index + 1);
        }
        out.push_str("</spine></package>\n");
        out
    }
}

/// An XHTML page's start, through `<body>`.
fn page_head(title: &str, language: &str) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE html>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\" lang=\"",
    );
    escape_attribute(&mut out, language);
    out.push_str("\" xml:lang=\"");
    escape_attribute(&mut out, language);
    out.push_str("\"><head><meta charset=\"UTF-8\"/><title>");
    escape_text(&mut out, title);
    out.push_str(
        "</title><link rel=\"stylesheet\" type=\"text/css\" href=\"../style.css\"/></head><body>",
    );
    out
}

/// The formatting a run adds to its paragraph's (`base`): the elements
/// to open, outermost first. Bold, italic, underline, and strike are tagged where the
/// run has them and `tagged` does not.
fn run_tags(
    run: &RunProperties,
    base: &RunProperties,
    tagged: &RunProperties,
    code: bool,
    base_size: f32,
    document: &Document,
) -> Vec<Element> {
    let mut elements: Vec<Element> = Vec::new();
    let mut tag = |name: &str| {
        elements.push(Element {
            open: format!("<{name}>"),
            close: format!("</{name}>"),
        });
    };
    let on = |value: Option<bool>| value == Some(true);
    if code {
        tag("code");
    }
    if on(run.bold) && !on(tagged.bold) {
        tag("strong");
    }
    if on(run.italic) && !on(tagged.italic) {
        tag("em");
    }
    if on(run.underline) && !on(tagged.underline) {
        tag("u");
    }
    if on(run.strike) && !on(tagged.strike) {
        tag("s");
    }
    if run.baseline != base.baseline {
        match run.baseline {
            Some(Baseline::Superscript) => tag("sup"),
            Some(Baseline::Subscript) => tag("sub"),
            None => {}
        }
    }
    // What tags cannot say: turning formatting off, and the rest.
    let mut style_off = RunProperties::default();
    if !on(run.bold) && on(base.bold) {
        style_off.bold = Some(false);
    }
    if !on(run.italic) && on(base.italic) {
        style_off.italic = Some(false);
    }
    let mut css = String::new();
    run_css(&style_off, None, base_size, document, &mut css);
    let mut changed = RunProperties::default();
    if run.font != base.font && !code {
        changed.font = run.font;
    }
    if run.size != base.size {
        changed.size = run.size;
    }
    if run.color != base.color {
        changed.color = run.color.or(Some(Color {
            red: 0,
            green: 0,
            blue: 0,
        }));
    }
    if run.highlight != base.highlight {
        changed.highlight = run.highlight;
    }
    if run.caps != base.caps {
        changed.caps = run.caps;
    }
    if run.letter_spacing != base.letter_spacing {
        changed.letter_spacing = run.letter_spacing;
    }
    run_css(&changed, base.size, base_size, document, &mut css);
    if !css.is_empty() {
        let mut span = String::from("<span style=\"");
        escape_attribute(&mut span, css.trim_end());
        span.push_str("\">");
        elements.insert(
            0,
            Element {
                open: span,
                close: "</span>".to_string(),
            },
        );
    }
    elements
}

/// Closes the open elements down to the first `keep`.
fn close_elements(stack: &mut Vec<Element>, keep: usize, out: &mut String) {
    while stack.len() > keep {
        if let Some(element) = stack.pop() {
            out.push_str(&element.close);
        }
    }
}

/// An inline element a run's formatting opens: its start and end tags.
#[derive(Clone, PartialEq)]
struct Element {
    open: String,
    close: String,
}

/// A paragraph's formatting as CSS declarations. `style_left` is the
/// left indent the paragraph's style gives, which a first-line indent of
/// its own is measured against when it does not state its own.
fn paragraph_css(
    properties: &ParagraphProperties,
    style_left: Option<f32>,
    base: f32,
    css: &mut String,
) {
    let em = |points: f32| format!("{}em", number(points / base));
    if let Some(alignment) = properties.alignment {
        let value = match alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "justify",
        };
        let _ = write!(css, "text-align: {value}; ");
    }
    if let Some(left) = properties.left_indent {
        let _ = write!(css, "margin-left: {}; ", em(left));
    }
    if let Some(right) = properties.right_indent {
        let _ = write!(css, "margin-right: {}; ", em(right));
    }
    if let Some(first) = properties.first_line_indent {
        let left = properties.left_indent.or(style_left).unwrap_or(0.0);
        let _ = write!(css, "text-indent: {}; ", em(first - left));
    }
    if let Some(before) = properties.space_before {
        let _ = write!(css, "margin-top: {}; ", em(before));
    }
    if let Some(after) = properties.space_after {
        let _ = write!(css, "margin-bottom: {}; ", em(after));
    }
    match properties.line_spacing {
        Some(LineSpacing::Relative(share)) if share > 0.0 => {
            let _ = write!(css, "line-height: {}; ", number(share.max(1.0) * 1.2));
        }
        Some(LineSpacing::Exact(points) | LineSpacing::Minimum(points)) if points > 0.0 => {
            let _ = write!(css, "line-height: {}; ", em(points));
        }
        _ => {}
    }
    if properties.keep_with_next == Some(true) {
        css.push_str("page-break-after: avoid; ");
    }
    if let Some(color) = properties.background {
        let _ = write!(css, "background-color: #{}; ", color.hex().to_lowercase());
    }
}

/// Character formatting as CSS declarations, sizes in `em` of `base` (or
/// of `relative_to`, the size the text sits in, when given).
fn run_css(
    run: &RunProperties,
    relative_to: Option<f32>,
    base: f32,
    document: &Document,
    css: &mut String,
) {
    if let Some(font) = run.font {
        let family = document.string(font).replace(['"', '\\'], "");
        if !family.is_empty() {
            let _ = write!(
                css,
                "font-family: \"{family}\", {}; ",
                generic_family(&family)
            );
        }
    }
    if let Some(size) = run.size {
        let _ = write!(
            css,
            "font-size: {}em; ",
            number(size / relative_to.unwrap_or(base))
        );
    }
    match run.bold {
        Some(true) => css.push_str("font-weight: bold; "),
        Some(false) => css.push_str("font-weight: normal; "),
        None => {}
    }
    match run.italic {
        Some(true) => css.push_str("font-style: italic; "),
        Some(false) => css.push_str("font-style: normal; "),
        None => {}
    }
    let mut decorations = Vec::new();
    if run.underline == Some(true) {
        decorations.push("underline");
    }
    if run.strike == Some(true) {
        decorations.push("line-through");
    }
    if !decorations.is_empty() {
        let _ = write!(css, "text-decoration: {}; ", decorations.join(" "));
    }
    if let Some(color) = run.color {
        let _ = write!(css, "color: #{}; ", color.hex().to_lowercase());
    }
    if let Some(color) = run.highlight {
        let _ = write!(css, "background-color: #{}; ", color.hex().to_lowercase());
    }
    match run.caps {
        Some(Caps::Small) => css.push_str("font-variant: small-caps; "),
        Some(Caps::All) => css.push_str("text-transform: uppercase; "),
        None => {}
    }
    if let Some(spacing) = run.letter_spacing
        && spacing != 0.0
    {
        let _ = write!(css, "letter-spacing: {}em; ", number(spacing / base));
    }
}

/// The CSS generic family a named font falls back to.
fn generic_family(family: &str) -> &'static str {
    let lower = family.to_ascii_lowercase();
    if ["mono", "courier", "consol", "menlo", "code"]
        .iter()
        .any(|word| lower.contains(word))
    {
        "monospace"
    } else if [
        "sans",
        "arial",
        "helvetica",
        "calibri",
        "verdana",
        "segoe",
        "nunito",
        "roboto",
        "open",
        "lato",
        "inter",
        "avenir",
        "proxima",
        "gill",
        "futura",
        "tahoma",
    ]
    .iter()
    .any(|word| lower.contains(word))
    {
        "sans-serif"
    } else {
        "serif"
    }
}

/// A number in CSS: at most three decimals, no trailing zeros.
fn number(value: f32) -> String {
    let text = format!("{value:.3}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    if text == "-0" {
        "0".to_string()
    } else {
        text.to_string()
    }
}

/// Text without the characters XML cannot carry.
fn clean(text: &str) -> String {
    text.chars()
        .filter(|ch| !matches!(*ch, '\u{0}'..='\u{8}' | '\u{b}' | '\u{c}' | '\u{e}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}'))
        .collect()
}

/// An equation's MathML, in the MathML namespace EPUB requires.
fn math(mathml: &str, out: &mut String) {
    let trimmed = mathml.trim();
    if !trimmed.starts_with("<math") && !trimmed.starts_with("<m:math") {
        escape_text(out, trimmed);
        return;
    }
    if trimmed.contains("xmlns=\"http://www.w3.org/1998/Math/MathML\"") {
        out.push_str(trimmed);
    } else if let Some(rest) = trimmed.strip_prefix("<math") {
        out.push_str("<math xmlns=\"http://www.w3.org/1998/Math/MathML\"");
        out.push_str(rest);
    } else {
        out.push_str(trimmed);
    }
}

/// The media type of a picture EPUB carries (its core media types), by
/// its file name.
fn media_type(name: &str) -> Option<&'static str> {
    let extension = name.rsplit('.').next()?.to_ascii_lowercase();
    match extension.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "svg" => Some("image/svg+xml"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// An embedded font's path in the package.
fn font_path(index: usize, font: &crate::document::EmbeddedFont) -> String {
    let family: String = font
        .family
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let face = match (font.bold, font.italic) {
        (false, false) => "Regular",
        (true, false) => "Bold",
        (false, true) => "Italic",
        (true, true) => "BoldItalic",
    };
    let extension = if font.bytes.starts_with(b"OTTO") {
        "otf"
    } else {
        "ttf"
    };
    format!("fonts/{family}-{face}-{}.{extension}", index + 1)
}

/// The modification time EPUB requires (`dcterms:modified`): now, to the
/// second, in UTC. WebAssembly has no clock, so its books carry a fixed
/// time.
fn modified() -> String {
    #[cfg(not(target_family = "wasm"))]
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    #[cfg(target_family = "wasm")]
    let seconds = 946_684_800i64;
    let days = seconds.div_euclid(86_400);
    let time = seconds.rem_euclid(86_400);
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_index = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_index + 2) / 5 + 1;
    let month = if month_index < 10 {
        month_index + 3
    } else {
        month_index - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        time / 3600,
        time % 3600 / 60,
        time % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_are_short() {
        assert_eq!(number(1.0), "1");
        assert_eq!(number(0.6666), "0.667");
        assert_eq!(number(-0.0001), "0");
    }

    #[test]
    fn the_modified_time_has_epubs_form() {
        let time = modified();
        assert_eq!(time.len(), 20, "{time}");
        assert!(time.ends_with('Z') && time.as_bytes()[10] == b'T', "{time}");
    }
}
