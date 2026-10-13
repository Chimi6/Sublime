//! Writing the document model as OpenDocument Text: `mimetype` stored
//! first, the manifest, `content.xml` (automatic styles for direct
//! formatting, then the body), `styles.xml` (fonts, named paragraph,
//! character, and list styles, page layouts, and a master page for each
//! page setup with its headers and footers), `meta.xml`, the pictures,
//! and each formula as an embedded formula object.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;

use crate::document::{
    Alignment, Baseline, Block, Border, Caps, Cell, Color, Document, FloatingContent, Inline,
    LineSpacing, ListLabel, ListStyle, Merge, NumberKind, PageSetup, PageVariants, Paragraph,
    ParagraphProperties, Placement, RevisionKind, Run, RunProperties, Section, SectionStart,
    StyleId, TabAlignment, Table, VerticalAlignment,
};
use crate::format::Effort;
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

const NAMESPACES: &str = concat!(
    r#"xmlns:office="urn:oasis:names:tc:opendocument:xmlns:office:1.0" "#,
    r#"xmlns:style="urn:oasis:names:tc:opendocument:xmlns:style:1.0" "#,
    r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0" "#,
    r#"xmlns:table="urn:oasis:names:tc:opendocument:xmlns:table:1.0" "#,
    r#"xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
    r#"xmlns:fo="urn:oasis:names:tc:opendocument:xmlns:xsl-fo-compatible:1.0" "#,
    r#"xmlns:xlink="http://www.w3.org/1999/xlink" "#,
    r#"xmlns:dc="http://purl.org/dc/elements/1.1/" "#,
    r#"xmlns:meta="urn:oasis:names:tc:opendocument:xmlns:meta:1.0" "#,
    r#"xmlns:svg="urn:oasis:names:tc:opendocument:xmlns:svg-compatible:1.0" "#,
    r#"xmlns:loext="urn:org:documentfoundation:names:experimental:office:xmlns:loext:1.0" "#,
    r#"office:version="1.3""#
);

/// Writes `document` as an `.odt` to `sink`.
pub fn write_odt<W: io::Write>(document: &Document, sink: W, effort: Effort) -> io::Result<W> {
    let mut writer = OdtWriter::new(document);
    let body = writer.body();
    let content = writer.content_xml(&body);
    let styles = writer.styles_xml();
    let mut zip = ZipWriter::new(sink).with_effort(effort);
    zip.add("mimetype", b"application/vnd.oasis.opendocument.text")?;
    zip.add_deflated("content.xml", content.as_bytes())?;
    zip.add_deflated("styles.xml", styles.as_bytes())?;
    zip.add_deflated("meta.xml", META.as_bytes())?;
    zip.add_deflated(
        "settings.xml",
        settings_xml(!document.fonts.is_empty()).as_bytes(),
    )?;
    for (index, font) in document.fonts.iter().enumerate() {
        zip.add_deflated(&font_path(index, font), &font.bytes)?;
    }
    for (path, media) in &writer.pictures {
        zip.add(path, &document.media[*media].bytes)?;
    }
    for (index, mathml) in writer.formulas.iter().enumerate() {
        let content = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{}",
            with_math_namespace(mathml)
        );
        zip.add_deflated(
            &format!("Object {}/content.xml", index + 1),
            content.as_bytes(),
        )?;
    }
    zip.add_deflated("META-INF/manifest.xml", writer.manifest().as_bytes())?;
    zip.finish()
}

const META: &str = concat!(
    "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
    "<office:document-meta xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
    "xmlns:meta=\"urn:oasis:names:tc:opendocument:xmlns:meta:1.0\" office:version=\"1.3\">",
    "<office:meta><meta:generator>Sublime</meta:generator></office:meta></office:document-meta>"
);

/// The layout rules LibreOffice takes for a Word document, which the model
/// follows: it sets these (all but `TabsRelativeToIndent`, since the writer
/// gives tab stops from the indent) when it opens a `.docx`. A setting a
/// document leaves out takes an older version's value, so the two line
/// spacing rules whose old values differ are stated too (`AddExternalLeading`,
/// `UseFormerLineSpacing`), and the layout is independent of the printer
/// (at its low resolution: the high one sets Word's line heights shorter,
/// three pages short in fifty-two, measured against LibreOffice's own
/// rendering of the Word files).
const WORD_LAYOUT: &[(&str, bool)] = &[
    ("AddExternalLeading", true),
    ("AddParaTableSpacing", false),
    ("AddVerticalFrameOffsets", true),
    ("AdjustTableLineHeightsToGridHeight", false),
    ("ApplyParagraphMarkFormatToEmptyLineAtEndOfParagraph", true),
    ("ApplyParagraphMarkFormatToNumbering", true),
    ("BackgroundParaOverDrawings", true),
    ("ClippedPictures", true),
    ("ConsiderTextWrapOnObjPos", true),
    ("ContinuousEndnotes", true),
    ("DisableOffPagePositioning", true),
    ("DoNotMirrorRtlDrawObjs", true),
    ("FrameAutowidthWithMorePara", true),
    ("HeaderSpacingBelowLastPara", true),
    ("IgnoreTabsAndBlanksForLineCalculation", true),
    ("InvertBorderSpacing", true),
    ("JustifyLinesWithShrinking", true),
    ("MinRowHeightInclBorder", true),
    ("MsWordCompGridMetrics", true),
    ("MsWordCompTrailingBlanks", true),
    ("NoClippingWithWrapPolygon", true),
    ("NoGapAfterNoteNumber", true),
    ("NoNumberingShowFollowBy", true),
    ("PaintHellOverHeaderFooter", true),
    ("SurroundTextWrapSmall", true),
    ("TabAtLeftIndentForParagraphsInList", true),
    ("TabOverSpacing", true),
    ("TableRowKeep", true),
    ("TreatSingleColumnBreakAsPageBreak", true),
    ("UnbreakableNumberings", true),
    ("UseFormerLineSpacing", false),
];

/// `settings.xml`: the Word layout rules, and whether LibreOffice should
/// load the embedded fonts. The set's name needs the `ooo` namespace, or
/// LibreOffice ignores it.
fn settings_xml(embed_fonts: bool) -> String {
    let mut items = String::from(
        "<config:config-item config:name=\"PrinterIndependentLayout\" config:type=\"string\">low-resolution</config:config-item>",
    );
    let embed = [("EmbedFonts", true)];
    let extra: &[(&str, bool)] = if embed_fonts { &embed } else { &[] };
    for (name, value) in WORD_LAYOUT.iter().chain(extra) {
        let _ = write!(
            items,
            "<config:config-item config:name=\"{name}\" config:type=\"boolean\">{value}</config:config-item>"
        );
    }
    format!(
        concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n",
            "<office:document-settings xmlns:office=\"urn:oasis:names:tc:opendocument:xmlns:office:1.0\" ",
            "xmlns:config=\"urn:oasis:names:tc:opendocument:xmlns:config:1.0\" ",
            "xmlns:ooo=\"http://openoffice.org/2004/office\" office:version=\"1.3\">",
            "<office:settings><config:config-item-set config:name=\"ooo:configuration-settings\">{}",
            "</config:config-item-set></office:settings></office:document-settings>"
        ),
        items
    )
}

/// An embedded font's path in the package.
fn font_path(index: usize, font: &crate::document::EmbeddedFont) -> String {
    let family: String = font
        .family
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '_'
            }
        })
        .collect();
    format!("Fonts/{family}_{}.ttf", index + 1)
}

/// A formula's MathML with the namespace a formula object needs.
fn with_math_namespace(mathml: &str) -> String {
    if mathml.contains("xmlns") {
        mathml.to_string()
    } else {
        mathml.replacen(
            "<math",
            "<math xmlns=\"http://www.w3.org/1998/Math/MathML\"",
            1,
        )
    }
}

/// A tracked change: its id, kind, author, date, and a deletion's text.
struct ChangeRegion {
    id: String,
    kind: RevisionKind,
    author: Option<String>,
    date: Option<String>,
    deleted: String,
}

/// An automatic paragraph style's identity: its parent, its direct
/// paragraph and run properties, its master page, and its break.
#[derive(Clone, PartialEq)]
struct ParagraphKey {
    parent: Option<StyleId>,
    paragraph: ParagraphProperties,
    run: RunProperties,
    master: Option<usize>,
    column_break: bool,
}

struct OdtWriter<'d> {
    document: &'d Document,
    fonts: Vec<String>,
    paragraph_styles: Vec<ParagraphKey>,
    text_styles: Vec<(Option<StyleId>, RunProperties)>,
    /// Each section's master page: the index of an equal one earlier.
    masters: Vec<usize>,
    /// Pictures by media index: their path in the package.
    pictures: Vec<(String, usize)>,
    picture_paths: HashMap<usize, String>,
    formulas: Vec<String>,
    tables: Vec<String>,
    /// Automatic table, column, and cell styles, as XML.
    table_styles: String,
    graphic_styles: String,
    frames: usize,
    notes: usize,
    /// Tracked changes, in order, and the model's revision ids to them.
    changes: Vec<ChangeRegion>,
    change_ids: HashMap<u32, usize>,
    /// Sections whose columns are written as text sections.
    column_sections: String,
    column_section_count: usize,
    /// The next paragraph starts a new page or column (a break that was
    /// a paragraph of its own).
    pending_break: Option<bool>,
}

impl<'d> OdtWriter<'d> {
    fn new(document: &'d Document) -> OdtWriter<'d> {
        let mut writer = OdtWriter {
            document,
            fonts: Vec::new(),
            paragraph_styles: Vec::new(),
            text_styles: Vec::new(),
            masters: Vec::new(),
            pictures: Vec::new(),
            picture_paths: HashMap::new(),
            formulas: Vec::new(),
            tables: Vec::new(),
            table_styles: String::new(),
            graphic_styles: String::new(),
            frames: 0,
            notes: 0,
            changes: Vec::new(),
            change_ids: HashMap::new(),
            column_sections: String::new(),
            column_section_count: 0,
            pending_break: None,
        };
        // Sections with the same page share a master page.
        let sections = &document.sections;
        for (index, section) in sections.iter().enumerate() {
            let same = (0..index).find(|&earlier| {
                same_page(&sections[earlier], section) && writer.masters[earlier] == earlier
            });
            writer.masters.push(same.unwrap_or(index));
        }
        writer
    }

    // ----- the body -----

    fn body(&mut self) -> String {
        let mut out = String::new();
        let document = self.document;
        // Floating objects no paragraph anchors sit on their pages.
        let mut anchored = std::collections::HashSet::new();
        for section in &document.sections {
            collect_anchors(&section.blocks, &mut anchored);
        }
        for (id, object) in document.floating.iter().enumerate() {
            if !anchored.contains(&(id as u32)) {
                self.floating(object, &mut out, Some(object.page + 1));
            }
        }
        let mut previous_master = None;
        for (index, section) in document.sections.iter().enumerate() {
            let master = self.masters[index];
            // A section on a new page with another page names its master
            // page on its first paragraph; one that changes only columns
            // continues as a text section.
            let new_master = previous_master != Some(master);
            let columns = section.columns > 1 && section.start == SectionStart::Continuous;
            if columns {
                self.column_section_count += 1;
                let name = format!("Sect{}", self.column_section_count);
                let _ = write!(
                    self.column_sections,
                    "<style:style style:name=\"{name}\" style:family=\"section\"><style:section-properties><style:columns fo:column-count=\"{}\" fo:column-gap=\"{}\"/></style:section-properties></style:style>",
                    section.columns,
                    points(section.column_gap.unwrap_or(36.0))
                );
                let _ = write!(
                    out,
                    "<text:section text:style-name=\"{name}\" text:name=\"{name}\">"
                );
            }
            let mut first = new_master.then_some(master);
            if section.blocks.is_empty() && first.is_some() {
                // An empty section still takes its page.
                let key = ParagraphKey {
                    parent: None,
                    paragraph: ParagraphProperties::default(),
                    run: RunProperties::default(),
                    master: first.take(),
                    column_break: false,
                };
                let style = self.paragraph_style(key);
                let _ = write!(out, "<text:p text:style-name=\"P{style}\"/>");
            }
            self.blocks(&section.blocks, &mut out, &mut first);
            if columns {
                out.push_str("</text:section>");
            }
            previous_master = Some(master);
        }
        out
    }

    /// Blocks, the first paragraph taking `master` (a master page change).
    fn blocks(&mut self, blocks: &[Block], out: &mut String, master: &mut Option<usize>) {
        let mut index = 0;
        while index < blocks.len() {
            match &blocks[index] {
                Block::Paragraph(paragraph) if paragraph.list.is_some() => {
                    let end = blocks[index..]
                        .iter()
                        .position(
                            |block| !matches!(block, Block::Paragraph(next) if next.list.is_some()),
                        )
                        .map_or(blocks.len(), |offset| index + offset);
                    self.list(&blocks[index..end], out, master);
                    index = end;
                }
                Block::Paragraph(paragraph) => {
                    self.paragraph(paragraph, out, master.take());
                    index += 1;
                }
                Block::Table(table) => {
                    if let Some(page) = master.take() {
                        // A table cannot name a master page: an empty
                        // paragraph before it does.
                        let style = self.paragraph_style(ParagraphKey {
                            parent: None,
                            paragraph: ParagraphProperties::default(),
                            run: RunProperties::default(),
                            master: Some(page),
                            column_break: false,
                        });
                        let _ = write!(out, "<text:p text:style-name=\"P{style}\"/>");
                    }
                    self.table(table, out);
                    index += 1;
                }
            }
        }
    }

    /// Consecutive list paragraphs as nested `text:list`s.
    fn list(&mut self, items: &[Block], out: &mut String, master: &mut Option<usize>) {
        // Open lists: (level, style); a list item is open when its list is.
        let mut open: Vec<StyleId> = Vec::new();
        for block in items {
            let Block::Paragraph(paragraph) = block else {
                continue;
            };
            let item = paragraph.list.expect("list paragraphs only");
            let level = item.level as usize;
            // Close deeper lists, and the whole list where another style
            // or a restart begins.
            let restart = item.starts_list && level == 0;
            if restart {
                while open.pop().is_some() {
                    out.push_str("</text:list-item></text:list>");
                }
            }
            while open.len() > level + 1 {
                open.pop();
                out.push_str("</text:list-item></text:list>");
            }
            if open.len() == level + 1 {
                out.push_str("</text:list-item>");
            }
            while open.len() < level + 1 {
                if open.is_empty() {
                    let _ = write!(
                        out,
                        "<text:list text:style-name=\"{}\"{}>",
                        list_style_name(item.style),
                        if item.starts_list {
                            ""
                        } else {
                            " text:continue-numbering=\"true\""
                        }
                    );
                } else {
                    // A level skipped (2 under 0) is an item without a
                    // paragraph between them.
                    if open.len() < level {
                        out.push_str("<text:list><text:list-item>");
                        open.push(item.style);
                        continue;
                    }
                    out.push_str("<text:list>");
                }
                open.push(item.style);
                if open.len() < level + 1 {
                    out.push_str("<text:list-item>");
                }
            }
            out.push_str("<text:list-item");
            if item.starts_list && item.start != 1 {
                let _ = write!(out, " text:start-value=\"{}\"", item.start);
            }
            // An item of another list style in the same list keeps its own.
            if open.first().is_some_and(|style| *style != item.style) {
                let _ = write!(
                    out,
                    " text:style-override=\"{}\"",
                    list_style_name(item.style)
                );
            }
            out.push('>');
            self.paragraph(paragraph, out, master.take());
        }
        while open.pop().is_some() {
            out.push_str("</text:list-item></text:list>");
        }
    }

    fn paragraph(&mut self, paragraph: &Paragraph, out: &mut String, master: Option<usize>) {
        // ODF has no break inside a paragraph: a paragraph is split at
        // each page or column break, the part after taking the break
        // (as LibreOffice reads Word's), and a break at the end becomes
        // the next paragraph's.
        let breaks = paragraph
            .runs
            .iter()
            .position(|run| matches!(run.content, Inline::PageBreak | Inline::ColumnBreak));
        if let Some(at) = breaks {
            let page = matches!(paragraph.runs[at].content, Inline::PageBreak);
            let before = Paragraph {
                runs: paragraph.runs[..at].to_vec(),
                ..paragraph.clone()
            };
            let after = Paragraph {
                runs: paragraph.runs[at + 1..].to_vec(),
                list: None,
                ..paragraph.clone()
            };
            let mut master = master;
            if !before.runs.is_empty() || master.is_some() {
                self.paragraph_part(&before, out, master.take());
            }
            self.pending_break = Some(page);
            if !after.runs.is_empty() {
                self.paragraph(&after, out, master);
            }
            return;
        }
        self.paragraph_part(paragraph, out, master);
    }

    fn paragraph_part(&mut self, paragraph: &Paragraph, out: &mut String, master: Option<usize>) {
        let document = self.document;
        let mut properties = document.paragraph_properties(paragraph);
        let column_break = match self.pending_break.take() {
            Some(true) => {
                properties.page_break_before = Some(true);
                false
            }
            Some(false) => true,
            None => false,
        };
        let outline = properties
            .outline_level
            .or_else(|| paragraph.style.and_then(|style| self.style_outline(style)));
        let key = ParagraphKey {
            parent: paragraph.style.or(document.styles.default_paragraph),
            paragraph: properties,
            run: document.paragraph_run_properties(paragraph),
            master,
            column_break,
        };
        let style = if key.paragraph.is_empty()
            && key.run.is_empty()
            && key.master.is_none()
            && !key.column_break
        {
            paragraph
                .style
                .or(document.styles.default_paragraph)
                .map_or_else(|| "Standard".to_string(), |id| self.named_paragraph(id))
        } else {
            format!("P{}", self.paragraph_style(key))
        };
        let element = if outline.is_some() {
            "text:h"
        } else {
            "text:p"
        };
        let _ = write!(out, "<{element} text:style-name=\"{style}\"");
        if let Some(level) = outline {
            let _ = write!(out, " text:outline-level=\"{}\"", level + 1);
        }
        out.push('>');
        self.runs(&paragraph.runs, out);
        let _ = write!(out, "</{element}>");
    }

    fn style_outline(&self, style: StyleId) -> Option<u8> {
        let mut current = Some(style);
        let mut guard = 0;
        while let Some(id) = current {
            let named = self.document.styles.paragraph.get(id)?;
            if let Some(level) = named.paragraph.outline_level {
                return Some(level);
            }
            current = named.parent;
            guard += 1;
            if guard > 32 {
                break;
            }
        }
        None
    }

    /// A named paragraph style's ODF name: the document's default style is
    /// `Standard`, which every other style and plain paragraph inherits.
    fn named_paragraph(&self, id: StyleId) -> String {
        if self.document.styles.default_paragraph == Some(id) {
            return "Standard".to_string();
        }
        self.document
            .styles
            .paragraph
            .get(id)
            .map_or_else(|| "Standard".to_string(), |style| style_name(&style.name))
    }

    fn paragraph_style(&mut self, key: ParagraphKey) -> usize {
        if let Some(index) = self
            .paragraph_styles
            .iter()
            .position(|existing| *existing == key)
        {
            return index + 1;
        }
        self.paragraph_styles.push(key);
        self.paragraph_styles.len()
    }

    fn text_style(&mut self, named: Option<StyleId>, run: RunProperties) -> usize {
        if let Some(index) = self
            .text_styles
            .iter()
            .position(|existing| existing.0 == named && existing.1 == run)
        {
            return index + 1;
        }
        self.text_styles.push((named, run));
        self.text_styles.len()
    }

    fn runs(&mut self, runs: &[Run], out: &mut String) {
        let document = self.document;
        let mut index = 0;
        // Leading spaces and runs of spaces are kept with `text:s`.
        let mut after_space = true;
        while index < runs.len() {
            let run = &runs[index];
            // A link spans its runs.
            if let Some(link) = document.link(run) {
                let end = runs[index..]
                    .iter()
                    .position(|other| document.link(other) != Some(link))
                    .map_or(runs.len(), |offset| index + offset);
                out.push_str("<text:a xlink:type=\"simple\" xlink:href=\"");
                escape_attribute(out, link);
                out.push_str("\">");
                for inner in &runs[index..end] {
                    self.run(inner, out, &mut after_space);
                }
                out.push_str("</text:a>");
                index = end;
                continue;
            }
            self.run(run, out, &mut after_space);
            index += 1;
        }
    }

    fn run(&mut self, run: &Run, out: &mut String, after_space: &mut bool) {
        let document = self.document;
        // A tracked change: an insertion marks its text; a deletion is a
        // marker here and its text in the change region.
        let change = run.revision.and_then(|id| self.change(id));
        if let Some(index) = change {
            let id = self.changes[index].id.clone();
            if self.changes[index].kind == RevisionKind::Deletion {
                if let Inline::Text(span) = run.content {
                    self.changes[index].deleted.push_str(document.text(span));
                }
                // One marker for a deletion's runs: at its first.
                if self.changes[index].deleted.len() == run_text_len(document, run) {
                    let _ = write!(out, "<text:change text:change-id=\"{id}\"/>");
                }
                return;
            }
            let _ = write!(out, "<text:change-start text:change-id=\"{id}\"/>");
        }
        let properties = document.run_properties(run);
        let styled = run.style.is_some() || !properties.is_empty();
        if styled {
            let style = self.text_style(run.style, properties);
            let _ = write!(out, "<text:span text:style-name=\"T{style}\">");
        }
        match run.content {
            Inline::Text(span) => write_text(out, document.text(span), after_space),
            Inline::LineBreak => {
                out.push_str("<text:line-break/>");
                *after_space = true;
            }
            Inline::Tab => {
                out.push_str("<text:tab/>");
                *after_space = true;
            }
            // Split off by `paragraph`.
            Inline::PageBreak | Inline::ColumnBreak => {}
            Inline::Math(span) => {
                self.formulas.push(document.text(span).to_string());
                self.frames += 1;
                let _ = write!(
                    out,
                    "<draw:frame draw:name=\"Formula{}\" text:anchor-type=\"as-char\"><draw:object xlink:href=\"./Object {}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>",
                    self.frames,
                    self.formulas.len()
                );
                *after_space = false;
            }
            Inline::Footnote(note) => {
                self.notes += 1;
                let _ = write!(
                    out,
                    "<text:note text:id=\"ftn{}\" text:note-class=\"footnote\"><text:note-citation>{}</text:note-citation><text:note-body>",
                    self.notes, self.notes
                );
                if let Some(note) = document.footnotes.get(note) {
                    let mut none = None;
                    self.blocks(&note.blocks, out, &mut none);
                }
                out.push_str("</text:note-body></text:note>");
                *after_space = false;
            }
            Inline::Image(id) => {
                if let Some(image) = document.image(id) {
                    self.image(image, out);
                }
                *after_space = false;
            }
            Inline::PageNumber => {
                out.push_str("<text:page-number text:select-page=\"current\">1</text:page-number>");
                *after_space = false;
            }
            Inline::PageCount => {
                out.push_str("<text:page-count>1</text:page-count>");
                *after_space = false;
            }
            Inline::Anchor(id) => {
                if let Some(object) = document.floating.get(id as usize) {
                    self.floating(object, out, None);
                }
            }
            Inline::CommentStart(id) => self.comment(id, out),
            Inline::CommentEnd(id) => {
                let _ = write!(out, "<office:annotation-end office:name=\"c{}\"/>", id + 1);
            }
        }
        if styled {
            out.push_str("</text:span>");
        }
        if let Some(index) = change {
            let _ = write!(
                out,
                "<text:change-end text:change-id=\"{}\"/>",
                self.changes[index].id
            );
        }
    }

    /// The change region for a model revision, made on first use.
    fn change(&mut self, revision: u32) -> Option<usize> {
        if let Some(index) = self.change_ids.get(&revision) {
            return Some(*index);
        }
        let info = self.document.revisions.get(revision as usize)?;
        let index = self.changes.len();
        self.changes.push(ChangeRegion {
            id: format!("ct{}", index + 1),
            kind: info.kind,
            author: info.author.clone(),
            date: info.date.clone(),
            deleted: String::new(),
        });
        self.change_ids.insert(revision, index);
        Some(index)
    }

    fn comment(&mut self, id: u32, out: &mut String) {
        let document = self.document;
        let Some(comment) = document.comments.get(id as usize) else {
            return;
        };
        let _ = write!(out, "<office:annotation office:name=\"c{}\">", id + 1);
        annotation_body(comment, out);
        out.push_str("</office:annotation>");
        // Replies follow their parent, sharing its range.
        for (index, reply) in document.comments.iter().enumerate() {
            if reply.reply_to == Some(id) {
                let _ = write!(
                    out,
                    "<office:annotation office:name=\"c{}\" loext:parent-name=\"c{}\">",
                    index + 1,
                    id + 1
                );
                annotation_body(reply, out);
                out.push_str("</office:annotation>");
            }
        }
    }

    fn picture_path(&mut self, media: usize) -> String {
        if let Some(path) = self.picture_paths.get(&media) {
            return path.clone();
        }
        let name = &self.document.media[media].name;
        let safe: String = name
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                    character
                } else {
                    '_'
                }
            })
            .collect();
        // The source's name, numbered only where two would collide.
        let mut path = format!("Pictures/{safe}");
        let mut counter = 1;
        while self.pictures.iter().any(|(taken, _)| *taken == path) {
            counter += 1;
            path = format!("Pictures/{counter}-{safe}");
        }
        self.pictures.push((path.clone(), media));
        self.picture_paths.insert(media, path.clone());
        path
    }

    fn image(&mut self, image: &crate::document::InlineImage, out: &mut String) {
        let path = self.picture_path(image.media);
        self.frames += 1;
        let (anchor, position) = match image.placement {
            Placement::Inline => ("as-char", String::new()),
            Placement::Floating {
                horizontal,
                vertical,
            } => (
                if matches!(vertical.from, crate::document::AnchorBase::Page) {
                    "page"
                } else {
                    "char"
                },
                format!(
                    " svg:x=\"{}\" svg:y=\"{}\"",
                    points(horizontal.offset),
                    points(vertical.offset)
                ),
            ),
        };
        let _ = write!(
            out,
            "<draw:frame draw:name=\"Image{}\" text:anchor-type=\"{anchor}\" svg:width=\"{}\" svg:height=\"{}\"{position}><draw:image xlink:href=\"{path}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/>",
            self.frames,
            points(image.width),
            points(image.height)
        );
        if let Some(description) = &image.description {
            out.push_str("<svg:desc>");
            escape_text(out, description);
            out.push_str("</svg:desc>");
        }
        out.push_str("</draw:frame>");
    }

    /// A floating object as a frame: anchored where it is written, or to
    /// `page` (one-based) when given.
    fn floating(
        &mut self,
        object: &crate::document::FloatingObject,
        out: &mut String,
        page: Option<u32>,
    ) {
        self.frames += 1;
        let frame = self.frames;
        let anchor = match page {
            Some(page) => format!("text:anchor-type=\"page\" text:anchor-page-number=\"{page}\""),
            None => "text:anchor-type=\"char\"".to_string(),
        };
        match &object.content {
            FloatingContent::Image(id) => {
                if let Some(image) = self.document.image(*id as u32) {
                    let path = self.picture_path(image.media);
                    let _ = write!(
                        out,
                        "<draw:frame draw:name=\"Image{frame}\" {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\"><draw:image xlink:href=\"{path}\" xlink:type=\"simple\" xlink:show=\"embed\" xlink:actuate=\"onLoad\"/></draw:frame>",
                        points(object.x),
                        points(object.y),
                        points(object.width),
                        points(object.height)
                    );
                }
            }
            FloatingContent::TextBox { blocks, .. } => {
                let _ = write!(
                    out,
                    "<draw:frame draw:name=\"Frame{frame}\" {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\"><draw:text-box>",
                    points(object.x),
                    points(object.y),
                    points(object.width),
                    points(object.height)
                );
                let mut none = None;
                self.blocks(blocks, out, &mut none);
                out.push_str("</draw:text-box></draw:frame>");
            }
            FloatingContent::Chart(chart) => {
                // A chart's data as a table in a text box: its values are
                // kept, its drawing is not.
                let _ = write!(
                    out,
                    "<draw:frame draw:name=\"Chart{frame}\" {anchor} svg:x=\"{}\" svg:y=\"{}\" svg:width=\"{}\" svg:height=\"{}\"><draw:text-box><table:table table:name=\"Chart{frame}\">",
                    points(object.x),
                    points(object.y),
                    points(object.width),
                    points(object.height)
                );
                let _ = write!(
                    out,
                    "<table:table-column table:number-columns-repeated=\"{}\"/>",
                    chart.series.len() + 1
                );
                let cell = |out: &mut String, text: &str| {
                    out.push_str("<table:table-cell office:value-type=\"string\"><text:p>");
                    let mut space = true;
                    write_text(out, text, &mut space);
                    out.push_str("</text:p></table:table-cell>");
                };
                out.push_str("<table:table-header-rows><table:table-row>");
                cell(out, "");
                for series in &chart.series {
                    cell(out, &series.name);
                }
                out.push_str("</table:table-row></table:table-header-rows>");
                for (index, category) in chart.categories.iter().enumerate() {
                    out.push_str("<table:table-row>");
                    cell(out, category);
                    for series in &chart.series {
                        let value = series
                            .values
                            .get(index)
                            .copied()
                            .flatten()
                            .map(|value| value.to_string())
                            .unwrap_or_default();
                        cell(out, &value);
                    }
                    out.push_str("</table:table-row>");
                }
                out.push_str("</table:table></draw:text-box></draw:frame>");
            }
        }
    }

    // ----- tables -----

    fn table(&mut self, table: &Table, out: &mut String) {
        let name = format!("Table{}", self.tables.len() + 1);
        self.tables.push(name.clone());
        let width: f32 = table.columns.iter().sum();
        let _ = write!(
            self.table_styles,
            "<style:style style:name=\"{name}\" style:family=\"table\"><style:table-properties"
        );
        if width > 0.0 {
            let _ = write!(self.table_styles, " style:width=\"{}\"", points(width));
        }
        let align = match table.alignment {
            Some(Alignment::Center) => "center",
            Some(Alignment::Right) => "right",
            _ if table.indent.is_some() => "left",
            _ => "margins",
        };
        let _ = write!(self.table_styles, " table:align=\"{align}\"");
        if let Some(indent) = table.indent {
            let _ = write!(self.table_styles, " fo:margin-left=\"{}\"", points(indent));
        }
        self.table_styles.push_str("/></style:style>");
        let _ = write!(
            out,
            "<table:table table:name=\"{name}\" table:style-name=\"{name}\">"
        );
        let columns = table.columns.len().max(
            table
                .rows
                .iter()
                .map(|row| row.cells.len())
                .max()
                .unwrap_or(0),
        );
        for column in 0..columns {
            let style = format!("{name}.C{}", column + 1);
            if let Some(width) = table.columns.get(column).filter(|width| **width > 0.0) {
                let _ = write!(
                    self.table_styles,
                    "<style:style style:name=\"{style}\" style:family=\"table-column\"><style:table-column-properties style:column-width=\"{}\"/></style:style>",
                    points(*width)
                );
                let _ = write!(out, "<table:table-column table:style-name=\"{style}\"/>");
            } else {
                out.push_str("<table:table-column/>");
            }
        }
        let header = table.header_rows as usize;
        for (row_index, row) in table.rows.iter().enumerate() {
            if row_index == 0 && header > 0 {
                out.push_str("<table:table-header-rows>");
            }
            if let Some(height) = row.height {
                let style = format!("{name}.R{}", row_index + 1);
                let _ = write!(
                    self.table_styles,
                    "<style:style style:name=\"{style}\" style:family=\"table-row\"><style:table-row-properties style:min-row-height=\"{}\"/></style:style>",
                    points(height)
                );
                let _ = write!(out, "<table:table-row table:style-name=\"{style}\">");
            } else {
                out.push_str("<table:table-row>");
            }
            for (column, cell) in row.cells.iter().enumerate() {
                self.cell(table, &name, row_index, column, cell, out);
            }
            for _ in row.cells.len()..columns {
                out.push_str(
                    "<table:table-cell office:value-type=\"string\"><text:p/></table:table-cell>",
                );
            }
            out.push_str("</table:table-row>");
            if header > 0 && row_index + 1 == header {
                out.push_str("</table:table-header-rows>");
            }
        }
        out.push_str("</table:table>");
    }

    fn cell(
        &mut self,
        table: &Table,
        name: &str,
        row: usize,
        column: usize,
        cell: &Cell,
        out: &mut String,
    ) {
        if cell.merge != Merge::Origin {
            out.push_str("<table:covered-table-cell/>");
            return;
        }
        let style = format!("{name}.{}{}", column_letters(column), row + 1);
        let mut properties = String::new();
        if let Some(background) = cell.background {
            let _ = write!(
                properties,
                " fo:background-color=\"#{}\"",
                background.hex().to_lowercase()
            );
        }
        let borders = table.borders.unwrap_or_default();
        let sides = [
            (
                "fo:border-top",
                cell.borders.top,
                if row == 0 {
                    borders.top
                } else {
                    borders.inside_horizontal
                },
            ),
            (
                "fo:border-bottom",
                cell.borders.bottom,
                if row + 1 == table.rows.len() {
                    borders.bottom
                } else {
                    borders.inside_horizontal
                },
            ),
            (
                "fo:border-left",
                cell.borders.left,
                if column == 0 {
                    borders.left
                } else {
                    borders.inside_vertical
                },
            ),
            (
                "fo:border-right",
                cell.borders.right,
                if column + 1 == table.rows[row].cells.len() {
                    borders.right
                } else {
                    borders.inside_vertical
                },
            ),
        ];
        let stated = table.borders.is_some();
        for (attribute_name, own, grid) in sides {
            let line = own.unwrap_or(grid);
            match line {
                Some(line) => {
                    let _ = write!(properties, " {attribute_name}=\"{}\"", border_text(line));
                }
                None if own.is_some() || stated => {
                    let _ = write!(properties, " {attribute_name}=\"none\"");
                }
                None => {
                    // No borders stated: a thin grid, as LibreOffice and
                    // Word draw a plain table.
                    let _ = write!(properties, " {attribute_name}=\"0.5pt solid #000000\"");
                }
            }
        }
        let margins = cell.margins;
        let default_margins = table.cell_margins;
        for (index, attribute_name) in [
            "fo:padding-top",
            "fo:padding-bottom",
            "fo:padding-left",
            "fo:padding-right",
        ]
        .iter()
        .enumerate()
        {
            let fallback = default_margins
                .map(|margins| [margins.top, margins.bottom, margins.left, margins.right][index]);
            if let Some(value) = margins[index].or(fallback) {
                let _ = write!(properties, " {attribute_name}=\"{}\"", points(value));
            }
        }
        if let Some(alignment) = cell.vertical_alignment {
            let value = match alignment {
                VerticalAlignment::Top => "top",
                VerticalAlignment::Center => "middle",
                VerticalAlignment::Bottom => "bottom",
            };
            let _ = write!(properties, " style:vertical-align=\"{value}\"");
        }
        let _ = write!(
            self.table_styles,
            "<style:style style:name=\"{style}\" style:family=\"table-cell\"><style:table-cell-properties{properties}/></style:style>"
        );
        let _ = write!(
            out,
            "<table:table-cell table:style-name=\"{style}\" office:value-type=\"string\""
        );
        if cell.column_span > 1 {
            let _ = write!(
                out,
                " table:number-columns-spanned=\"{}\"",
                cell.column_span
            );
        }
        if cell.row_span > 1 {
            let _ = write!(out, " table:number-rows-spanned=\"{}\"", cell.row_span);
        }
        out.push('>');
        if cell.blocks.is_empty() {
            out.push_str("<text:p/>");
        } else {
            let mut none = None;
            self.blocks(&cell.blocks, out, &mut none);
        }
        out.push_str("</table:table-cell>");
    }

    // ----- the parts -----

    fn font(&mut self, run: &RunProperties) -> Option<String> {
        let name = self.document.string(run.font?).to_string();
        if name.is_empty() {
            return None;
        }
        if !self.fonts.contains(&name) {
            self.fonts.push(name.clone());
        }
        Some(name)
    }

    fn text_properties(&mut self, run: &RunProperties) -> String {
        let mut out = String::new();
        if let Some(font) = self.font(run) {
            out.push_str(" style:font-name=\"");
            escape_attribute(&mut out, &font);
            out.push('"');
        }
        let language = run.language.map(|id| self.document.string(id).to_string());
        text_attributes(run, language.as_deref(), &mut out);
        out
    }

    fn content_xml(&mut self, body: &str) -> String {
        let mut automatic = String::new();
        for index in 0..self.paragraph_styles.len() {
            let key = self.paragraph_styles[index].clone();
            let _ = write!(
                automatic,
                "<style:style style:name=\"P{}\" style:family=\"paragraph\"",
                index + 1
            );
            if let Some(parent) = key.parent {
                let parent = self.named_paragraph(parent);
                let _ = write!(automatic, " style:parent-style-name=\"{parent}\"");
            } else {
                automatic.push_str(" style:parent-style-name=\"Standard\"");
            }
            if let Some(master) = key.master {
                let _ = write!(
                    automatic,
                    " style:master-page-name=\"{}\"",
                    master_name(master)
                );
            }
            automatic.push('>');
            let mut paragraph = paragraph_attributes(&key.paragraph);
            if key.column_break {
                paragraph.push_str(" fo:break-before=\"column\"");
            }
            let tabs = self.tab_stops(&key.paragraph);
            if !paragraph.is_empty() || !tabs.is_empty() {
                let _ = write!(automatic, "<style:paragraph-properties{paragraph}");
                if tabs.is_empty() {
                    automatic.push_str("/>");
                } else {
                    let _ = write!(automatic, ">{tabs}</style:paragraph-properties>");
                }
            }
            let text = self.text_properties(&key.run);
            if !text.is_empty() {
                let _ = write!(automatic, "<style:text-properties{text}/>");
            }
            automatic.push_str("</style:style>");
        }
        for index in 0..self.text_styles.len() {
            let (named, run) = self.text_styles[index];
            let _ = write!(
                automatic,
                "<style:style style:name=\"T{}\" style:family=\"text\"",
                index + 1
            );
            if let Some(named) = named.and_then(|id| self.document.styles.character.get(id)) {
                let _ = write!(
                    automatic,
                    " style:parent-style-name=\"{}\"",
                    style_name(&named.name)
                );
            }
            let text = self.text_properties(&run);
            let _ = write!(automatic, "><style:text-properties{text}/></style:style>");
        }
        automatic.push_str(&self.table_styles);
        automatic.push_str(&self.graphic_styles);
        automatic.push_str(&self.column_sections);
        let mut changes = String::new();
        if !self.changes.is_empty() {
            changes.push_str("<text:tracked-changes>");
            for change in &self.changes {
                let kind = match change.kind {
                    RevisionKind::Insertion => "text:insertion",
                    RevisionKind::Deletion => "text:deletion",
                };
                let _ = write!(
                    changes,
                    "<text:changed-region xml:id=\"{0}\" text:id=\"{0}\"><{kind}><office:change-info><dc:creator>",
                    change.id
                );
                escape_text(&mut changes, change.author.as_deref().unwrap_or("Unknown"));
                changes.push_str("</dc:creator><dc:date>");
                escape_text(
                    &mut changes,
                    change
                        .date
                        .as_deref()
                        .map_or("1970-01-01T00:00:00", |date| date.trim_end_matches('Z')),
                );
                changes.push_str("</dc:date></office:change-info>");
                if change.kind == RevisionKind::Deletion {
                    changes.push_str("<text:p>");
                    let mut space = true;
                    write_text(&mut changes, &change.deleted, &mut space);
                    changes.push_str("</text:p>");
                }
                let _ = write!(changes, "</{kind}></text:changed-region>");
            }
            changes.push_str("</text:tracked-changes>");
        }
        let fonts = self.font_decls();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-content {NAMESPACES}><office:font-face-decls>{fonts}</office:font-face-decls><office:automatic-styles>{automatic}</office:automatic-styles><office:body><office:text>{changes}{body}</office:text></office:body></office:document-content>"
        )
    }

    fn tab_stops(&self, paragraph: &ParagraphProperties) -> String {
        let stops = self.document.tab_set(paragraph.tabs);
        if stops.is_empty() {
            return String::new();
        }
        let indent = paragraph.left_indent.unwrap_or(0.0);
        let mut out = String::from("<style:tab-stops>");
        for stop in stops {
            let kind = match stop.alignment {
                TabAlignment::Left => "left",
                TabAlignment::Center => "center",
                TabAlignment::Right => "right",
                TabAlignment::Decimal => "char",
            };
            let _ = write!(
                out,
                "<style:tab-stop style:position=\"{}\" style:type=\"{kind}\"",
                points(stop.position - indent)
            );
            if stop.alignment == TabAlignment::Decimal {
                out.push_str(" style:char=\".\"");
            }
            if let Some(leader) = stop.leader {
                out.push_str(" style:leader-style=\"solid\" style:leader-text=\"");
                escape_attribute(&mut out, &leader.to_string());
                out.push('"');
            }
            out.push_str("/>");
        }
        out.push_str("</style:tab-stops>");
        out
    }

    fn font_decls(&self) -> String {
        let mut out = String::new();
        let mut families: Vec<&str> = self.fonts.iter().map(String::as_str).collect();
        // Embedded families are declared even where no style names them.
        for font in &self.document.fonts {
            if !families.contains(&font.family.as_str()) {
                families.push(&font.family);
            }
        }
        for family in families {
            out.push_str("<style:font-face style:name=\"");
            escape_attribute(&mut out, family);
            out.push_str("\" svg:font-family=\"'");
            escape_attribute(&mut out, family);
            out.push('\'');
            out.push('"');
            let faces: Vec<(usize, &crate::document::EmbeddedFont)> = self
                .document
                .fonts
                .iter()
                .enumerate()
                .filter(|(_, font)| font.family == family)
                .collect();
            if faces.is_empty() {
                out.push_str("/>");
                continue;
            }
            out.push_str("><svg:font-face-src>");
            for (index, font) in faces {
                let _ = write!(
                    out,
                    "<svg:font-face-uri xlink:href=\"{}\" xlink:type=\"simple\" loext:font-style=\"{}\" loext:font-weight=\"{}\"><svg:font-face-format svg:string=\"truetype\"/></svg:font-face-uri>",
                    font_path(index, font),
                    if font.italic { "italic" } else { "normal" },
                    if font.bold { "bold" } else { "normal" }
                );
            }
            out.push_str("</svg:font-face-src></style:font-face>");
        }
        out
    }

    fn styles_xml(&mut self) -> String {
        let document = self.document;
        let mut named = String::new();
        named.push_str("<style:default-style style:family=\"paragraph\"><style:paragraph-properties style:writing-mode=\"page\"/><style:text-properties fo:font-size=\"12pt\"/></style:default-style>");
        let has_standard = document.styles.default_paragraph.is_some()
            || document
                .styles
                .paragraph
                .iter()
                .any(|style| style_name(&style.name) == "Standard");
        if !has_standard {
            named.push_str("<style:style style:name=\"Standard\" style:family=\"paragraph\" style:class=\"text\"/>");
        }
        for (index, style) in document.styles.paragraph.iter().enumerate() {
            let name = self.named_paragraph(index);
            // Another style named Standard yields to the default.
            if name != "Standard"
                && style_name(&style.name) == "Standard"
                && document.styles.default_paragraph.is_some()
            {
                continue;
            }
            named.push_str("<style:style style:name=\"");
            named.push_str(&name);
            named.push_str("\" style:display-name=\"");
            escape_attribute(&mut named, &style.name);
            named.push_str("\" style:family=\"paragraph\"");
            match style
                .parent
                .filter(|parent| *parent < document.styles.paragraph.len())
            {
                Some(parent) if self.named_paragraph(parent) != name => {
                    let _ = write!(
                        named,
                        " style:parent-style-name=\"{}\"",
                        self.named_paragraph(parent)
                    );
                }
                _ if name != "Standard" => named.push_str(" style:parent-style-name=\"Standard\""),
                _ => {}
            }
            if let Some(level) = style.paragraph.outline_level {
                let _ = write!(named, " style:default-outline-level=\"{}\"", level + 1);
            }
            named.push('>');
            let paragraph = paragraph_attributes(&style.paragraph);
            let tabs = self.tab_stops(&style.paragraph);
            if !paragraph.is_empty() || !tabs.is_empty() {
                let _ = write!(named, "<style:paragraph-properties{paragraph}");
                if tabs.is_empty() {
                    named.push_str("/>");
                } else {
                    let _ = write!(named, ">{tabs}</style:paragraph-properties>");
                }
            }
            let text = self.text_properties(&style.run);
            if !text.is_empty() {
                let _ = write!(named, "<style:text-properties{text}/>");
            }
            named.push_str("</style:style>");
        }
        for style in &document.styles.character {
            let name = style_name(&style.name);
            let text = self.text_properties(&style.run);
            named.push_str("<style:style style:name=\"");
            named.push_str(&name);
            named.push_str("\" style:display-name=\"");
            escape_attribute(&mut named, &style.name);
            let _ = write!(
                named,
                "\" style:family=\"text\"><style:text-properties{text}/></style:style>"
            );
        }
        for (index, list) in document.styles.list.iter().enumerate() {
            list_style_xml(index, list, &mut named);
        }
        // Page layouts and master pages, one for each distinct page.
        let mut layouts = String::new();
        let mut masters = String::new();
        // LibreOffice paints a page's colour from its master's drawing-page
        // style, and other readers from the page layout: both are written.
        let fill = match document.page_color {
            Some(color) => {
                let _ = write!(
                    layouts,
                    "<style:style style:name=\"dp1\" style:family=\"drawing-page\"><style:drawing-page-properties draw:fill=\"solid\" draw:fill-color=\"#{}\" draw:background-size=\"full\"/></style:style>",
                    color.hex().to_lowercase()
                );
                " draw:style-name=\"dp1\""
            }
            None => "",
        };
        for (index, section) in document.sections.iter().enumerate() {
            if self.masters[index] != index {
                continue;
            }
            let number = index + 1;
            page_layout_xml(
                number,
                &section.page,
                section,
                document.page_color,
                &mut layouts,
            );
            let _ = write!(
                masters,
                "<style:master-page style:name=\"{}\" style:display-name=\"{}\" style:page-layout-name=\"pm{number}\"{fill}>",
                master_name(index),
                if index == 0 {
                    "Default".to_string()
                } else {
                    format!("Page {number}")
                }
            );
            self.page_parts(&section.headers, "header", &mut masters);
            self.page_parts(&section.footers, "footer", &mut masters);
            masters.push_str("</style:master-page>");
        }
        if document.sections.is_empty() {
            page_layout_xml(
                1,
                &PageSetup::default(),
                &Section::default(),
                document.page_color,
                &mut layouts,
            );
            let _ = write!(
                masters,
                "<style:master-page style:name=\"Standard\" style:page-layout-name=\"pm1\"{fill}/>"
            );
        }
        let fonts = self.font_decls();
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<office:document-styles {NAMESPACES}><office:font-face-decls>{fonts}</office:font-face-decls><office:styles>{named}</office:styles><office:automatic-styles>{layouts}</office:automatic-styles><office:master-styles>{masters}</office:master-styles></office:document-styles>"
        )
    }

    fn page_parts(&mut self, parts: &PageVariants, kind: &str, out: &mut String) {
        let write_part = |element: &str,
                          blocks: &Option<Vec<Block>>,
                          writer: &mut OdtWriter<'_>,
                          out: &mut String| {
            if let Some(blocks) = blocks {
                let _ = write!(out, "<style:{element}>");
                let mut none = None;
                let mut inner = String::new();
                writer.blocks(blocks, &mut inner, &mut none);
                out.push_str(&inner);
                let _ = write!(out, "</style:{element}>");
            }
        };
        write_part(kind, &parts.default, self, out);
        write_part(&format!("{kind}-left"), &parts.even, self, out);
        write_part(&format!("{kind}-first"), &parts.first, self, out);
    }

    fn manifest(&self) -> String {
        let mut out = String::from(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<manifest:manifest xmlns:manifest=\"urn:oasis:names:tc:opendocument:xmlns:manifest:1.0\" manifest:version=\"1.3\"><manifest:file-entry manifest:full-path=\"/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.text\"/><manifest:file-entry manifest:full-path=\"content.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"styles.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"meta.xml\" manifest:media-type=\"text/xml\"/><manifest:file-entry manifest:full-path=\"settings.xml\" manifest:media-type=\"text/xml\"/>",
        );
        for (path, media) in &self.pictures {
            let kind = media_type(&self.document.media[*media].name);
            let _ = write!(
                out,
                "<manifest:file-entry manifest:full-path=\"{path}\" manifest:media-type=\"{kind}\"/>"
            );
        }
        for (index, font) in self.document.fonts.iter().enumerate() {
            let _ = write!(
                out,
                "<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"application/x-font-ttf\"/>",
                font_path(index, font)
            );
        }
        for index in 0..self.formulas.len() {
            let _ = write!(
                out,
                "<manifest:file-entry manifest:full-path=\"Object {0}/\" manifest:version=\"1.3\" manifest:media-type=\"application/vnd.oasis.opendocument.formula\"/><manifest:file-entry manifest:full-path=\"Object {0}/content.xml\" manifest:media-type=\"text/xml\"/>",
                index + 1
            );
        }
        out.push_str("</manifest:manifest>");
        out
    }
}

/// The floating objects anchored in `blocks`' runs.
fn collect_anchors(blocks: &[Block], anchored: &mut std::collections::HashSet<u32>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => {
                for run in &paragraph.runs {
                    if let Inline::Anchor(id) = run.content {
                        anchored.insert(id);
                    }
                }
            }
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_anchors(&cell.blocks, anchored);
                    }
                }
            }
        }
    }
}

/// A run's text length (0 for other content).
fn run_text_len(document: &Document, run: &Run) -> usize {
    match run.content {
        Inline::Text(span) => document.text(span).len(),
        _ => 0,
    }
}

fn annotation_body(comment: &crate::document::Comment, out: &mut String) {
    out.push_str("<dc:creator>");
    escape_text(out, &comment.author);
    out.push_str("</dc:creator>");
    if let Some(date) = &comment.date {
        out.push_str("<dc:date>");
        escape_text(out, date.trim_end_matches('Z'));
        out.push_str("</dc:date>");
    }
    for line in comment.text.split('\n') {
        out.push_str("<text:p>");
        let mut space = true;
        write_text(out, line, &mut space);
        out.push_str("</text:p>");
    }
}

/// Text with ODF's white space kept: a space after a space (or at the
/// start) is `text:s`, tabs and newlines their elements.
fn write_text(out: &mut String, text: &str, after_space: &mut bool) {
    let mut spaces = 0usize;
    let flush = |out: &mut String, spaces: &mut usize, after_space: &mut bool| {
        if *spaces == 0 {
            return;
        }
        let mut count = *spaces;
        if !*after_space {
            out.push(' ');
            count -= 1;
        }
        if count == 1 {
            out.push_str("<text:s/>");
        } else if count > 1 {
            let _ = write!(out, "<text:s text:c=\"{count}\"/>");
        }
        *spaces = 0;
        *after_space = true;
    };
    for character in text.chars() {
        match character {
            ' ' => spaces += 1,
            '\t' => {
                flush(out, &mut spaces, after_space);
                out.push_str("<text:tab/>");
                *after_space = true;
            }
            '\n' => {
                flush(out, &mut spaces, after_space);
                out.push_str("<text:line-break/>");
                *after_space = true;
            }
            '\r' => {}
            _ => {
                flush(out, &mut spaces, after_space);
                let mut buffer = [0u8; 4];
                escape_text(out, character.encode_utf8(&mut buffer));
                *after_space = false;
            }
        }
    }
    flush(out, &mut spaces, after_space);
}

fn paragraph_attributes(paragraph: &ParagraphProperties) -> String {
    let mut out = String::new();
    if let Some(alignment) = paragraph.alignment {
        let value = match alignment {
            Alignment::Left => "start",
            Alignment::Center => "center",
            Alignment::Right => "end",
            Alignment::Justify => "justify",
        };
        let _ = write!(out, " fo:text-align=\"{value}\"");
    }
    if let Some(left) = paragraph.left_indent {
        let _ = write!(out, " fo:margin-left=\"{}\"", points(left));
    }
    if let Some(right) = paragraph.right_indent {
        let _ = write!(out, " fo:margin-right=\"{}\"", points(right));
    }
    if let Some(first) = paragraph.first_line_indent {
        let _ = write!(
            out,
            " fo:text-indent=\"{}\"",
            points(first - paragraph.left_indent.unwrap_or(0.0))
        );
    }
    if let Some(before) = paragraph.space_before {
        let _ = write!(out, " fo:margin-top=\"{}\"", points(before));
    }
    if let Some(after) = paragraph.space_after {
        let _ = write!(out, " fo:margin-bottom=\"{}\"", points(after));
    }
    match paragraph.line_spacing {
        Some(LineSpacing::Relative(share)) => {
            let _ = write!(out, " fo:line-height=\"{}%\"", (share * 100.0).round());
        }
        Some(LineSpacing::Exact(height)) => {
            let _ = write!(out, " fo:line-height=\"{}\"", points(height));
        }
        Some(LineSpacing::Minimum(height)) => {
            let _ = write!(out, " style:line-height-at-least=\"{}\"", points(height));
        }
        None => {}
    }
    if paragraph.keep_with_next == Some(true) {
        out.push_str(" fo:keep-with-next=\"always\"");
    }
    if paragraph.keep_lines_together == Some(true) {
        out.push_str(" fo:keep-together=\"always\"");
    }
    match paragraph.widow_control {
        Some(true) => out.push_str(" fo:widows=\"2\" fo:orphans=\"2\""),
        Some(false) => out.push_str(" fo:widows=\"0\" fo:orphans=\"0\""),
        None => {}
    }
    if let Some(background) = paragraph.background {
        let _ = write!(
            out,
            " fo:background-color=\"#{}\"",
            background.hex().to_lowercase()
        );
    }
    if paragraph.contextual_spacing == Some(true) {
        out.push_str(" style:contextual-spacing=\"true\"");
    }
    if let Some(border) = paragraph.border {
        let line = border_text(border.line);
        for (side, on) in [
            ("top", border.top),
            ("bottom", border.bottom),
            ("left", border.left),
            ("right", border.right),
        ] {
            let _ = write!(
                out,
                " fo:border-{side}=\"{}\"",
                if on { line.as_str() } else { "none" }
            );
        }
    }
    if paragraph.page_break_before == Some(true) {
        out.push_str(" fo:break-before=\"page\"");
    }
    out
}

fn text_attributes(run: &RunProperties, language: Option<&str>, out: &mut String) {
    if let Some(size) = run.size {
        let size = points(size);
        let _ = write!(
            out,
            " fo:font-size=\"{size}\" style:font-size-asian=\"{size}\" style:font-size-complex=\"{size}\""
        );
    }
    if let Some(bold) = run.bold {
        let weight = if bold { "bold" } else { "normal" };
        let _ = write!(
            out,
            " fo:font-weight=\"{weight}\" style:font-weight-asian=\"{weight}\" style:font-weight-complex=\"{weight}\""
        );
    }
    if let Some(italic) = run.italic {
        let style = if italic { "italic" } else { "normal" };
        let _ = write!(
            out,
            " fo:font-style=\"{style}\" style:font-style-asian=\"{style}\" style:font-style-complex=\"{style}\""
        );
    }
    if let Some(underline) = run.underline {
        if underline {
            out.push_str(" style:text-underline-style=\"solid\" style:text-underline-width=\"auto\" style:text-underline-color=\"font-color\"");
        } else {
            out.push_str(" style:text-underline-style=\"none\"");
        }
    }
    if let Some(strike) = run.strike {
        let style = if strike { "solid" } else { "none" };
        let _ = write!(out, " style:text-line-through-style=\"{style}\"");
    }
    if let Some(color) = run.color {
        let _ = write!(out, " fo:color=\"#{}\"", color.hex().to_lowercase());
    }
    if let Some(highlight) = run.highlight {
        let _ = write!(
            out,
            " fo:background-color=\"#{}\"",
            highlight.hex().to_lowercase()
        );
    }
    match run.baseline {
        Some(Baseline::Superscript) => out.push_str(" style:text-position=\"super 58%\""),
        Some(Baseline::Subscript) => out.push_str(" style:text-position=\"sub 58%\""),
        None => {
            if let (Some(shift), size) = (run.shift, run.size.unwrap_or(12.0)) {
                let percent = (shift / size.max(1.0) * 100.0).round();
                let _ = write!(out, " style:text-position=\"{percent}% 100%\"");
            }
        }
    }
    match run.caps {
        Some(Caps::Small) => out.push_str(" fo:font-variant=\"small-caps\""),
        Some(Caps::All) => out.push_str(" fo:text-transform=\"uppercase\""),
        None => {}
    }
    if run.hidden == Some(true) {
        out.push_str(" text:display=\"none\"");
    }
    if let Some(spacing) = run.letter_spacing {
        let _ = write!(out, " fo:letter-spacing=\"{}\"", points(spacing));
    }
    if let Some(scale) = run.width_scale {
        let _ = write!(out, " style:text-scale=\"{}%\"", scale.round());
    }
    if let Some(language) = language {
        let mut parts = language.splitn(2, '-');
        let code = parts.next().unwrap_or("");
        let _ = write!(out, " fo:language=\"{code}\"");
        if let Some(country) = parts.next() {
            let _ = write!(out, " fo:country=\"{country}\"");
        }
    }
}

fn list_style_xml(index: usize, list: &ListStyle, out: &mut String) {
    let _ = write!(
        out,
        "<text:list-style style:name=\"{}\" style:display-name=\"",
        list_style_name(index)
    );
    escape_attribute(out, &list.name);
    out.push_str("\">");
    // ODF list styles have ten levels.
    for level in 0..10 {
        let fallback = crate::document::ListLevel {
            label: ListLabel::Text("•".to_string()),
            indent: 18.0 * (level as f32 + 1.0),
            label_indent: 18.0 * level as f32,
        };
        let spec = list.levels.get(level).unwrap_or(&fallback);
        let number = level + 1;
        match &spec.label {
            ListLabel::Number(format) => {
                let (prefix, suffix) = format.pattern.split_once("%1").unwrap_or(("", "."));
                let kind = match format.kind {
                    NumberKind::Decimal => "1",
                    NumberKind::LowerLetter => "a",
                    NumberKind::UpperLetter => "A",
                    NumberKind::LowerRoman => "i",
                    NumberKind::UpperRoman => "I",
                };
                let _ = write!(
                    out,
                    "<text:list-level-style-number text:level=\"{number}\" style:num-format=\"{kind}\""
                );
                if !prefix.is_empty() {
                    out.push_str(" style:num-prefix=\"");
                    escape_attribute(out, prefix);
                    out.push('"');
                }
                if !suffix.is_empty() {
                    out.push_str(" style:num-suffix=\"");
                    escape_attribute(out, suffix);
                    out.push('"');
                }
                if format.tiered && level > 0 {
                    let _ = write!(out, " text:display-levels=\"{number}\"");
                }
                out.push('>');
                level_position(spec, out);
                out.push_str("</text:list-level-style-number>");
            }
            ListLabel::Text(bullet) => {
                let _ = write!(
                    out,
                    "<text:list-level-style-bullet text:level=\"{number}\" text:bullet-char=\""
                );
                escape_attribute(
                    out,
                    bullet
                        .chars()
                        .next()
                        .map(String::from)
                        .as_deref()
                        .unwrap_or("•"),
                );
                out.push_str("\">");
                level_position(spec, out);
                out.push_str("</text:list-level-style-bullet>");
            }
            ListLabel::None => {
                let _ = write!(
                    out,
                    "<text:list-level-style-number text:level=\"{number}\" style:num-format=\"\">"
                );
                level_position(spec, out);
                out.push_str("</text:list-level-style-number>");
            }
        }
    }
    out.push_str("</text:list-style>");
}

fn level_position(level: &crate::document::ListLevel, out: &mut String) {
    let _ = write!(
        out,
        "<style:list-level-properties text:list-level-position-and-space-mode=\"label-alignment\"><style:list-level-label-alignment text:label-followed-by=\"listtab\" text:list-tab-stop-position=\"{}\" fo:text-indent=\"{}\" fo:margin-left=\"{}\"/></style:list-level-properties>",
        points(level.indent),
        points(level.label_indent - level.indent),
        points(level.indent)
    );
}

fn page_layout_xml(
    number: usize,
    page: &PageSetup,
    section: &Section,
    background: Option<Color>,
    out: &mut String,
) {
    let header = !section.headers.is_empty();
    let footer = !section.footers.is_empty();
    // ODF's page margins reach the header and footer; the model's reach
    // the body, the header at its distance from the edge.
    let top = if header {
        page.header_distance
    } else {
        page.margin_top
    };
    let bottom = if footer {
        page.footer_distance
    } else {
        page.margin_bottom
    };
    let _ = write!(
        out,
        "<style:page-layout style:name=\"pm{number}\"><style:page-layout-properties fo:page-width=\"{}\" fo:page-height=\"{}\" style:print-orientation=\"{}\" fo:margin-top=\"{}\" fo:margin-bottom=\"{}\" fo:margin-left=\"{}\" fo:margin-right=\"{}\"",
        points(page.width),
        points(page.height),
        if page.width > page.height {
            "landscape"
        } else {
            "portrait"
        },
        points(top),
        points(bottom),
        points(page.margin_left),
        points(page.margin_right)
    );
    if let Some(first) = page.page_number_start {
        let _ = write!(out, " style:first-page-number=\"{first}\"");
    }
    if let Some(color) = background {
        let _ = write!(
            out,
            " fo:background-color=\"#{}\"",
            color.hex().to_lowercase()
        );
    }
    if section.columns > 1 && section.start == SectionStart::NewPage {
        let _ = write!(
            out,
            "><style:columns fo:column-count=\"{}\" fo:column-gap=\"{}\"/></style:page-layout-properties>",
            section.columns,
            points(section.column_gap.unwrap_or(36.0))
        );
    } else {
        out.push_str("/>");
    }
    if header {
        let height = (page.margin_top - page.header_distance).max(0.0);
        let _ = write!(
            out,
            "<style:header-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-bottom=\"0pt\"/></style:header-style>",
            points(height)
        );
    }
    if footer {
        let height = (page.margin_bottom - page.footer_distance).max(0.0);
        let _ = write!(
            out,
            "<style:footer-style><style:header-footer-properties fo:min-height=\"{}\" fo:margin-top=\"0pt\"/></style:footer-style>",
            points(height)
        );
    }
    out.push_str("</style:page-layout>");
}

/// Whether two sections share a page: size, margins, columns on the
/// page, and the same headers and footers.
fn same_page(left: &Section, right: &Section) -> bool {
    left.page == right.page
        && left.headers == right.headers
        && left.footers == right.footers
        && (left.columns == right.columns || right.start == SectionStart::Continuous)
}

fn list_style_name(index: StyleId) -> String {
    format!("L{}", index + 1)
}

/// A style's display name as an ODF style name: anything but letters,
/// digits, `-`, and `.` escaped as `_hex_` (`Heading 1` is
/// `Heading_20_1`).
/// A section's master page: the first is the document's `Standard`, which
/// a page without a named master takes.
fn master_name(index: usize) -> String {
    if index == 0 {
        "Standard".to_string()
    } else {
        format!("M{}", index + 1)
    }
}

fn style_name(name: &str) -> String {
    let mut out = String::new();
    for character in name.chars() {
        if character.is_ascii_alphanumeric() || matches!(character, '-' | '.') {
            out.push(character);
        } else {
            let _ = write!(out, "_{:x}_", character as u32);
        }
    }
    if out.starts_with(|character: char| {
        character.is_ascii_digit() || character == '-' || character == '.'
    }) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out.push_str("Style");
    }
    out
}

fn column_letters(column: usize) -> String {
    let mut letters = String::new();
    let mut number = column + 1;
    while number > 0 {
        let remainder = (number - 1) % 26;
        letters.insert(0, (b'A' + remainder as u8) as char);
        number = (number - 1) / 26;
    }
    letters
}

fn border_text(border: Border) -> String {
    let color = border.color.unwrap_or(Color {
        red: 0,
        green: 0,
        blue: 0,
    });
    format!(
        "{} solid #{}",
        points(border.width.max(0.05)),
        color.hex().to_lowercase()
    )
}

/// Points as an ODF length, without trailing zeros.
fn points(value: f32) -> String {
    let mut text = format!("{value:.3}");
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
    if text == "-0" {
        text = "0".to_string();
    }
    format!("{text}pt")
}

fn media_type(name: &str) -> &'static str {
    match name
        .rsplit('.')
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "svg" => "image/svg+xml",
        "bmp" => "image/bmp",
        "tif" | "tiff" => "image/tiff",
        "wmf" => "image/x-wmf",
        "emf" => "image/x-emf",
        _ => "application/octet-stream",
    }
}
