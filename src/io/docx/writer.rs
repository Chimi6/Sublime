//! Renders the document model as a `.docx` package: `document.xml`,
//! `styles.xml`, `numbering.xml`, `footnotes.xml`, `settings.xml`, the
//! header and footer parts, the media files, their relationships, and the
//! content types. Style names are kept, so a reader on the other side
//! (Word, Google Docs) sees the document's own style names.
//!
//! Units: the model's points become twentieths of a point for spacing and
//! indents, half-points for font sizes, and EMUs for drawings.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;

use crate::document::{
    Alignment, Anchor, AnchorBase, Baseline, Block, Caps, Document, FloatingContent, Inline,
    InlineImage, LineSpacing, ListLabel, Merge, NumberKind, PageKind, Paragraph,
    ParagraphProperties, Placement, RevisionKind, Run, RunProperties, Section, SectionStart, Table,
    TextWrap, VerticalAlignment, mathml_text,
};
use crate::io::deflate::Level;
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

/// The body is compressed into the package in parts of this size as it
/// renders, so a large document never holds its XML whole.
const BODY_PART: usize = 256 * 1024;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const R: &str = r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const DRAWING: &str = r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape""#;
const SHAPE: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const W14: &str = "http://schemas.microsoft.com/office/word/2010/wordml";
const W15: &str = "http://schemas.microsoft.com/office/word/2012/wordml";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const PICTURE: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// Writes `document` as a `.docx` to `sink`.
pub fn write_docx<W: io::Write>(document: &Document, sink: W) -> io::Result<W> {
    let mut writer = DocxWriter::new(document);
    let mut zip = ZipWriter::new(sink);
    // The body goes first, streamed; the parts it discovers (media,
    // headers, relationships) follow it. Entry order in a package is free.
    zip.begin_deflated("word/document.xml")?;
    writer.render_body(document, &mut |part| zip.write_part(part, Level::Fast))?;
    zip.end_deflated()?;
    write_parts(writer, zip, document)
}

/// A `.docx` written one top-level block at a time, for builders that
/// would rather not hold the whole document: the body streams into the
/// package as blocks arrive, and the parts that depend on the whole
/// (styles, numbering, footnotes, media) are written at the end from the
/// document as it stands then. The document's first section supplies the
/// page setup; floating objects, headers, and footers are not streamed.
pub struct DocxStream<W: io::Write> {
    writer: DocxWriter,
    zip: ZipWriter<W>,
    body: String,
}

impl<W: io::Write> DocxStream<W> {
    pub fn new(sink: W) -> io::Result<DocxStream<W>> {
        let mut zip = ZipWriter::new(sink);
        zip.begin_deflated("word/document.xml")?;
        let mut body = String::with_capacity(BODY_PART + 4096);
        body.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(body, "<w:document {W} {R} {DRAWING}><w:body>");
        Ok(DocxStream {
            writer: DocxWriter::new(&Document::default()),
            zip,
            body,
        })
    }

    /// Renders one block of the body. `document` is the arena and tables
    /// the block refers to, as they stand now.
    pub fn block(&mut self, document: &Document, block: &Block) -> io::Result<()> {
        match block {
            Block::Paragraph(paragraph) => {
                self.writer
                    .render_paragraph(document, paragraph, &mut self.body, "", &[]);
            }
            Block::Table(table) => self.writer.render_table(document, table, &mut self.body),
        }
        if self.body.len() >= BODY_PART {
            self.zip.write_part(self.body.as_bytes(), Level::Fast)?;
            self.body.clear();
        }
        Ok(())
    }

    /// Closes the body with the first section's properties and writes
    /// every other part.
    pub fn finish(mut self, document: &Document) -> io::Result<W> {
        let default_section = Section::default();
        let section = document.sections.first().unwrap_or(&default_section);
        let properties = self.writer.section_properties(document, section);
        self.body.push_str(&properties);
        self.body.push_str("</w:body></w:document>");
        self.zip.write_part(self.body.as_bytes(), Level::Fast)?;
        self.zip.end_deflated()?;
        write_parts(self.writer, self.zip, document)
    }
}

/// The comments part: each comment's author, date, and paragraphs.
fn comments_xml(document: &Document) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    let _ = write!(xml, "<w:comments {W} xmlns:w14=\"{W14}\">");
    for (id, comment) in document.comments.iter().enumerate() {
        let _ = write!(xml, "<w:comment w:id=\"{id}\" w:author=\"");
        escape_attribute(&mut xml, &comment.author);
        xml.push('"');
        if let Some(date) = &comment.date {
            xml.push_str(" w:date=\"");
            escape_attribute(&mut xml, date);
            xml.push('"');
        }
        if let Some(initials) = &comment.initials {
            xml.push_str(" w:initials=\"");
            escape_attribute(&mut xml, initials);
            xml.push('"');
        }
        xml.push('>');
        let lines: Vec<&str> = comment.text.split('\n').collect();
        for (index, line) in lines.iter().enumerate() {
            // The last paragraph is the comment's id in its thread.
            if index + 1 == lines.len() {
                let _ = write!(xml, "<w:p w14:paraId=\"{}\">", comment_paragraph(id));
            } else {
                xml.push_str("<w:p>");
            }
            xml.push_str("<w:r><w:t xml:space=\"preserve\">");
            escape_text(&mut xml, line);
            xml.push_str("</w:t></w:r></w:p>");
        }
        xml.push_str("</w:comment>");
    }
    xml.push_str("</w:comments>");
    xml
}

/// The paragraph id a comment is known by in its thread.
fn comment_paragraph(id: usize) -> String {
    format!("{:08X}", 0x0C00_0000 + id)
}

/// Word's comment threads: each comment's paragraph id, a reply's with its
/// parent's.
fn comments_extended_xml(document: &Document) -> String {
    let mut xml = String::new();
    xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
    let _ = write!(xml, "<w15:commentsEx xmlns:w15=\"{W15}\">");
    for (id, comment) in document.comments.iter().enumerate() {
        let _ = write!(
            xml,
            "<w15:commentEx w15:paraId=\"{}\"",
            comment_paragraph(id)
        );
        if let Some(parent) = comment.reply_to {
            let _ = write!(
                xml,
                " w15:paraIdParent=\"{}\"",
                comment_paragraph(parent as usize)
            );
        }
        xml.push_str(" w15:done=\"0\"/>");
    }
    xml.push_str("</w15:commentsEx>");
    xml
}

/// Every part but the body, from the writer's bookkeeping and the
/// document's tables.
fn write_parts<W: io::Write>(
    mut writer: DocxWriter,
    mut zip: ZipWriter<W>,
    document: &Document,
) -> io::Result<W> {
    let footnotes = if document.footnotes.is_empty() {
        None
    } else {
        Some(writer.footnotes_part(document))
    };
    zip.add_deflated(
        "[Content_Types].xml",
        writer.content_types(document).as_bytes(),
    )?;
    zip.add_deflated("_rels/.rels", root_relationships().as_bytes())?;
    zip.add_deflated(
        "word/_rels/document.xml.rels",
        writer
            .relationships_xml(document, &writer.relationships, true)
            .as_bytes(),
    )?;
    zip.add_deflated("word/styles.xml", writer.styles_xml(document).as_bytes())?;
    if !writer.numbering.instances.is_empty() {
        writer.write_numbering(document, &mut zip)?;
    }
    if let Some(footnotes) = &footnotes {
        zip.add_deflated("word/footnotes.xml", footnotes.xml.as_bytes())?;
        if !footnotes.relationships.is_empty() {
            zip.add_deflated(
                "word/_rels/footnotes.xml.rels",
                writer
                    .relationships_xml(document, &footnotes.relationships, false)
                    .as_bytes(),
            )?;
        }
    }
    if writer.needs_settings(document) {
        zip.add_deflated(
            "word/settings.xml",
            settings_xml(writer.even_pages, document.page_color.is_some()).as_bytes(),
        )?;
    }
    if !document.comments.is_empty() {
        zip.add_deflated("word/comments.xml", comments_xml(document).as_bytes())?;
        zip.add_deflated(
            "word/commentsExtended.xml",
            comments_extended_xml(document).as_bytes(),
        )?;
    }
    for part in &writer.page_parts {
        zip.add_deflated(&format!("word/{}", part.name), part.xml.as_bytes())?;
        if !part.relationships.is_empty() {
            zip.add_deflated(
                &format!("word/_rels/{}.rels", part.name),
                writer
                    .relationships_xml(document, &part.relationships, false)
                    .as_bytes(),
            )?;
        }
    }
    for (index, chart) in writer.charts.iter().enumerate() {
        zip.add_deflated(
            &format!("word/charts/chart{}.xml", index + 1),
            chart.as_bytes(),
        )?;
    }
    for (index, target) in writer.media_targets.iter().enumerate() {
        if let Some(target) = target {
            zip.add_deflated(&format!("word/{target}"), &document.media[index].bytes)?;
        }
    }
    zip.finish()
}

struct DocxWriter {
    body: String,
    /// The document part's relationships: `rId<n+10>`.
    relationships: Vec<Relationship>,
    numbering: Numbering,
    /// Header and footer parts, in order of creation.
    page_parts: Vec<Part>,
    /// The package path under `word/` of each media file that is used.
    media_targets: Vec<Option<String>>,
    /// Drawings written so far, for unique ids.
    drawings: u32,
    /// Chart parts (`charts/chart<n>.xml`), in order.
    charts: Vec<String>,
    /// Some section has an even-page header or footer.
    even_pages: bool,
    /// Which floating objects have been anchored to a paragraph.
    floating_done: Vec<bool>,
    /// Tracked changes written so far, for unique ids.
    revisions: u32,
    /// Character styles made from run formatting: key -> index into the
    /// list (`formatting_style`).
    formatting_styles: HashMap<u64, usize>,
    /// The named character style each is based on, and its formatting.
    formatting_style_list: Vec<(Option<usize>, RunProperties)>,
}

/// A relationship of a part: what it links to and how.
#[derive(PartialEq)]
struct Relationship {
    kind: RelationshipKind,
    target: String,
}

#[derive(PartialEq, Clone, Copy)]
enum RelationshipKind {
    Image,
    Header,
    Footer,
    Chart,
}

/// A part rendered on its own, with its own relationships.
struct Part {
    name: String,
    xml: String,
    relationships: Vec<Relationship>,
}

/// One `w:num` per list start: which list style it uses and the number it
/// starts at.
#[derive(Default)]
struct Numbering {
    instances: Vec<(usize, u32)>,
    /// The current instance for each list style, until a list restarts.
    current: Vec<Option<usize>>,
}

impl DocxWriter {
    fn new(document: &Document) -> DocxWriter {
        DocxWriter {
            body: String::new(),
            relationships: Vec::new(),
            numbering: Numbering::default(),
            page_parts: Vec::new(),
            media_targets: vec![None; document.media.len()],
            drawings: 0,
            charts: Vec::new(),
            even_pages: false,
            floating_done: vec![false; document.floating.len()],
            revisions: 0,
            formatting_styles: HashMap::new(),
            formatting_style_list: Vec::new(),
        }
    }

    /// Renders the document part, handing `flush` each part of it as
    /// `BODY_PART` fills, so the XML is never held whole.
    fn render_body(
        &mut self,
        document: &Document,
        flush: &mut dyn FnMut(&[u8]) -> io::Result<()>,
    ) -> io::Result<()> {
        let mut body = std::mem::take(&mut self.body);
        body.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(body, "<w:document {W} {R} {DRAWING}>");
        if let Some(color) = document.page_color {
            let _ = write!(body, "<w:background w:color=\"{}\"/>", color.hex());
        }
        body.push_str("<w:body>");
        // Floating objects are anchored to the first paragraph on their
        // page, counting the page breaks the document spells out.
        let mut page = 0u32;
        for (index, section) in document.sections.iter().enumerate() {
            let last = index + 1 == document.sections.len();
            if index > 0 && section.start == SectionStart::NewPage {
                page += 1;
            }
            let properties = self.section_properties(document, section);
            for (position, block) in section.blocks.iter().enumerate() {
                let is_last_block = position + 1 == section.blocks.len();
                match block {
                    Block::Paragraph(paragraph) => {
                        let breaks = paragraph
                            .runs
                            .iter()
                            .filter(|run| run.content == Inline::PageBreak)
                            .count();
                        page += breaks as u32;
                        let anchors = self.floating_due(document, page, last && is_last_block);
                        let trailer = if is_last_block && !last {
                            properties.as_str()
                        } else {
                            ""
                        };
                        self.render_paragraph(document, paragraph, &mut body, trailer, &anchors);
                    }
                    Block::Table(table) => self.render_table(document, table, &mut body),
                }
                if body.len() >= BODY_PART {
                    flush(body.as_bytes())?;
                    body.clear();
                }
            }
            // A section's properties ride in its last paragraph, as Word
            // and Pages both write them; a section ending otherwise gets
            // an empty one.
            let ends_with_paragraph = matches!(section.blocks.last(), Some(Block::Paragraph(_)));
            if !ends_with_paragraph {
                let anchors = self.floating_due(document, page, last);
                self.render_paragraph(
                    document,
                    &Paragraph::default(),
                    &mut body,
                    &properties,
                    &anchors,
                );
            } else if last {
                body.push_str(&properties);
            }
        }
        body.push_str("</w:body></w:document>");
        flush(body.as_bytes())?;
        body.clear();
        self.body = body;
        Ok(())
    }

    /// The floating objects to anchor now: those on `page` or before it
    /// that have no anchor yet, and every remaining one at the very end.
    fn floating_due(&mut self, document: &Document, page: u32, all: bool) -> Vec<usize> {
        let mut due = Vec::new();
        for (index, object) in document.floating.iter().enumerate() {
            // One moving with the text is written at its anchor run, one
            // repeating in its header or footer.
            if self.floating_done[index] || object.follows_text || object.repeats.is_some() {
                continue;
            }
            if all || object.page <= page {
                self.floating_done[index] = true;
                due.push(index);
            }
        }
        due
    }

    fn render_blocks(&mut self, document: &Document, blocks: &[Block], out: &mut String) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => {
                    self.render_paragraph(document, paragraph, out, "", &[])
                }
                Block::Table(table) => self.render_table(document, table, out),
            }
        }
    }

    #[inline(never)]
    fn render_table(&mut self, document: &Document, table: &Table, out: &mut String) {
        let column_width =
            |column: usize| twips(table.columns.get(column).copied().unwrap_or(100.0));
        let column_count = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(0)
            .max(table.columns.len());
        let total: i64 = (0..column_count).map(column_width).sum();
        let _ = write!(
            out,
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"{total}\" w:type=\"dxa\"/>"
        );
        match table.alignment {
            Some(Alignment::Center) => out.push_str("<w:jc w:val=\"center\"/>"),
            Some(Alignment::Right) => out.push_str("<w:jc w:val=\"right\"/>"),
            _ => {}
        }
        if let Some(indent) = table.indent {
            let _ = write!(out, "<w:tblInd w:w=\"{}\" w:type=\"dxa\"/>", twips(indent));
        }
        // The table's own lines and margins, over the grid style's.
        if let Some(borders) = table.borders {
            out.push_str("<w:tblBorders>");
            for (name, line) in [
                ("top", borders.top),
                ("left", borders.left),
                ("bottom", borders.bottom),
                ("right", borders.right),
                ("insideH", borders.inside_horizontal),
                ("insideV", borders.inside_vertical),
            ] {
                border_xml(name, Some(line), out);
            }
            out.push_str("</w:tblBorders>");
        }
        out.push_str("<w:tblLayout w:type=\"fixed\"/>");
        if let Some(margins) = table.cell_margins {
            out.push_str("<w:tblCellMar>");
            for (name, points) in [
                ("top", margins.top),
                ("left", margins.left),
                ("bottom", margins.bottom),
                ("right", margins.right),
            ] {
                let _ = write!(out, "<w:{name} w:w=\"{}\" w:type=\"dxa\"/>", twips(points));
            }
            out.push_str("</w:tblCellMar>");
        }
        out.push_str("</w:tblPr><w:tblGrid>");
        for column in 0..column_count {
            let _ = write!(out, "<w:gridCol w:w=\"{}\"/>", column_width(column));
        }
        out.push_str("</w:tblGrid>");
        for (row_index, row) in table.rows.iter().enumerate() {
            out.push_str("<w:tr>");
            let mut row_properties = String::new();
            if let Some(height) = row.height {
                let _ = write!(
                    row_properties,
                    "<w:trHeight w:val=\"{}\" w:hRule=\"atLeast\"/>",
                    twips(height)
                );
            }
            if (row_index as u32) < table.header_rows {
                row_properties.push_str("<w:tblHeader/>");
            }
            if !row_properties.is_empty() {
                let _ = write!(out, "<w:trPr>{row_properties}</w:trPr>");
            }
            for (column, cell) in row.cells.iter().enumerate() {
                if cell.merge == Merge::Left {
                    continue;
                }
                let span = cell.column_span.max(1) as usize;
                let width: i64 = (column..column + span).map(column_width).sum();
                let _ = write!(out, "<w:tc><w:tcPr><w:tcW w:w=\"{width}\" w:type=\"dxa\"/>");
                if span > 1 {
                    let _ = write!(out, "<w:gridSpan w:val=\"{span}\"/>");
                }
                match cell.merge {
                    Merge::Origin if cell.row_span > 1 => {
                        out.push_str("<w:vMerge w:val=\"restart\"/>");
                    }
                    Merge::Above => out.push_str("<w:vMerge/>"),
                    _ => {}
                }
                let sides = [
                    ("top", cell.borders.top),
                    ("left", cell.borders.left),
                    ("bottom", cell.borders.bottom),
                    ("right", cell.borders.right),
                ];
                if sides.iter().any(|(_, side)| side.is_some()) {
                    out.push_str("<w:tcBorders>");
                    for (name, side) in sides {
                        border_xml(name, side, out);
                    }
                    out.push_str("</w:tcBorders>");
                }
                if let Some(background) = cell.background {
                    let _ = write!(
                        out,
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                        background.hex()
                    );
                }
                let [top, bottom, left, right] = cell.margins;
                if cell.margins.iter().any(Option::is_some) {
                    out.push_str("<w:tcMar>");
                    for (name, points) in [
                        ("top", top),
                        ("left", left),
                        ("bottom", bottom),
                        ("right", right),
                    ] {
                        if let Some(points) = points {
                            let _ =
                                write!(out, "<w:{name} w:w=\"{}\" w:type=\"dxa\"/>", twips(points));
                        }
                    }
                    out.push_str("</w:tcMar>");
                }
                if let Some(vertical) = cell.vertical_alignment {
                    let value = match vertical {
                        VerticalAlignment::Top => "top",
                        VerticalAlignment::Center => "center",
                        VerticalAlignment::Bottom => "bottom",
                    };
                    let _ = write!(out, "<w:vAlign w:val=\"{value}\"/>");
                }
                out.push_str("</w:tcPr>");
                // A cell holds at least one paragraph, and ends with one.
                let ends_with_paragraph = matches!(cell.blocks.last(), Some(Block::Paragraph(_)));
                if cell.blocks.is_empty() {
                    out.push_str("<w:p/>");
                } else {
                    self.render_blocks(document, &cell.blocks, out);
                    if !ends_with_paragraph {
                        out.push_str("<w:p/>");
                    }
                }
                out.push_str("</w:tc>");
            }
            out.push_str("</w:tr>");
        }
        out.push_str("</w:tbl>");
    }

    /// `trailer` is extra paragraph-property XML, for a section's
    /// properties; `anchors` are the floating objects anchored here.
    fn render_paragraph(
        &mut self,
        document: &Document,
        paragraph: &Paragraph,
        out: &mut String,
        trailer: &str,
        anchors: &[usize],
    ) {
        out.push_str("<w:p>");
        let mut properties = String::new();
        if let Some(style) = paragraph.style {
            let _ = write!(
                properties,
                "<w:pStyle w:val=\"{}\"/>",
                paragraph_style_id(document, style)
            );
        }
        let numbering = paragraph.list.map(|item| {
            let instance = self
                .numbering
                .instance_for(item.style, item.starts_list, item.start);
            format!(
                "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                item.level,
                instance + 1
            )
        });
        let mut own = document.paragraph_properties(paragraph);
        if paragraph.list.is_some() {
            // A list paragraph is indented by its list level (Pages keeps the
            // paragraph's own indents at zero); stating them would override it.
            own.left_indent = None;
            own.first_line_indent = None;
        }
        paragraph_properties_xml(
            &own,
            numbering.as_deref(),
            document.tab_set(own.tabs),
            &mut properties,
        );
        let mut mark = String::new();
        let mut mark_properties = document.paragraph_run_properties(paragraph);
        mark_properties.overlay(&document.paragraph_mark_properties(paragraph));
        run_properties_xml(document, &mark_properties, &mut mark);
        if !mark.is_empty() {
            let _ = write!(properties, "<w:rPr>{mark}</w:rPr>");
        }
        properties.push_str(trailer);
        if !properties.is_empty() {
            out.push_str("<w:pPr>");
            out.push_str(&properties);
            out.push_str("</w:pPr>");
        }
        for anchor in anchors {
            self.render_floating(document, *anchor, out);
        }
        let mut open_link: Option<&str> = None;
        for run in &paragraph.runs {
            // Links are HYPERLINK fields, as Pages writes them: a field
            // carries its target itself, where a `w:hyperlink` needs a
            // relationship per target in the part's table (a document of a
            // hundred thousand links held eighty megabytes of them).
            if document.link(run) != open_link {
                if open_link.is_some() {
                    out.push_str("</w:fldSimple>");
                }
                open_link = document.link(run);
                if let Some(target) = open_link {
                    out.push_str("<w:fldSimple w:instr=\" HYPERLINK &quot;");
                    escape_attribute(out, &target.replace('"', "%22"));
                    out.push_str("&quot; \">");
                }
            }
            // A tracked change wraps its run.
            let change = document.revision(run).map(|revision| {
                self.revisions += 1;
                let element = match revision.kind {
                    RevisionKind::Insertion => "w:ins",
                    RevisionKind::Deletion => "w:del",
                };
                let mut author = String::new();
                escape_attribute(&mut author, revision.author.as_deref().unwrap_or("Unknown"));
                let mut date = String::new();
                if let Some(when) = &revision.date {
                    date.push_str(" w:date=\"");
                    escape_attribute(&mut date, when);
                    date.push('"');
                }
                let _ = write!(
                    out,
                    "<{element} w:id=\"{}\" w:author=\"{author}\"{date}>",
                    self.revisions
                );
                element
            });
            self.render_run(document, paragraph, run, out);
            if let Some(element) = change {
                let _ = write!(out, "</{element}>");
            }
        }
        if open_link.is_some() {
            out.push_str("</w:fldSimple>");
        }
        out.push_str("</w:p>");
    }

    fn render_run(
        &mut self,
        document: &Document,
        paragraph: &Paragraph,
        run: &Run,
        out: &mut String,
    ) {
        let mut properties = String::new();
        let mut merged = document.paragraph_run_properties(paragraph);
        merged.overlay(&document.run_properties(run));
        // Formatting that repeats across runs goes into a character style
        // once and the run carries its id; the toggle properties (bold,
        // italic, strike, caps) stay inline, since Word applies them
        // relative to the paragraph style when they come from a character
        // style. A run with a named character style and direct formatting
        // gets a style based on the named one, as Word allows one style
        // per run.
        let (styled, inline) = split_for_style(merged);
        if styled.is_empty() {
            if let Some(style) = run.style {
                let _ = write!(
                    properties,
                    "<w:rStyle w:val=\"{}\"/>",
                    style_id(&document.styles.character[style].name)
                );
            }
            run_properties_xml(document, &merged, &mut properties);
        } else {
            let style = self.formatting_style(paragraph, run, styled);
            let _ = write!(properties, "<w:rStyle w:val=\"r{style}\"/>");
            run_properties_xml(document, &inline, &mut properties);
        }
        let properties = if properties.is_empty() {
            String::new()
        } else {
            format!("<w:rPr>{properties}</w:rPr>")
        };
        // Page fields wrap their run.
        if let Inline::PageNumber | Inline::PageCount = run.content {
            let instruction = match run.content {
                Inline::PageCount => "NUMPAGES",
                _ => "PAGE",
            };
            let _ = write!(
                out,
                "<w:fldSimple w:instr=\" {instruction} \"><w:r>{properties}<w:t>1</w:t></w:r></w:fldSimple>"
            );
            return;
        }
        // A comment's range is marked between runs; its end carries the
        // reference Word shows the comment at.
        // A thread's replies share its range.
        let thread = |id: u32| {
            std::iter::once(id).chain(
                document
                    .comments
                    .iter()
                    .enumerate()
                    .filter(move |(_, comment)| comment.reply_to == Some(id))
                    .map(|(reply, _)| reply as u32),
            )
        };
        match run.content {
            Inline::CommentStart(id) => {
                for id in thread(id) {
                    let _ = write!(out, "<w:commentRangeStart w:id=\"{id}\"/>");
                }
                return;
            }
            Inline::CommentEnd(id) => {
                for id in thread(id) {
                    let _ = write!(
                        out,
                        "<w:commentRangeEnd w:id=\"{id}\"/><w:r><w:commentReference w:id=\"{id}\"/></w:r>"
                    );
                }
                return;
            }
            _ => {}
        }
        // An object moving with the text is drawn at its anchor.
        if let Inline::Anchor(index) = run.content {
            if document
                .floating
                .get(index as usize)
                .is_some_and(|object| object.follows_text)
            {
                self.render_floating(document, index as usize, out);
            }
            return;
        }
        out.push_str("<w:r>");
        out.push_str(&properties);
        let deleted = document.is_deleted(run);
        match run.content {
            Inline::Text(span) => {
                let element = if deleted { "w:delText" } else { "w:t" };
                let text = document.text(span);
                // Only edge whitespace needs the preserve attribute.
                let edged = text.starts_with(' ') || text.ends_with(' ');
                let _ = write!(out, "<{element}");
                if edged {
                    out.push_str(" xml:space=\"preserve\"");
                }
                out.push('>');
                escape_text(out, text);
                let _ = write!(out, "</{element}>");
            }
            Inline::PageBreak => out.push_str("<w:br w:type=\"page\"/>"),
            Inline::ColumnBreak => out.push_str("<w:br w:type=\"column\"/>"),
            // Equations are written as their text until Office Math is
            // supported.
            Inline::Math(span) => {
                out.push_str("<w:t xml:space=\"preserve\">");
                escape_text(out, &mathml_text(document.text(span)));
                out.push_str("</w:t>");
            }
            Inline::LineBreak => out.push_str("<w:br/>"),
            Inline::Tab => out.push_str("<w:tab/>"),
            Inline::Footnote(note) => {
                let _ = write!(out, "<w:footnoteReference w:id=\"{}\"/>", note + 1);
            }
            Inline::Image(id) => {
                if let Some(image) = document.image(id) {
                    self.render_image(document, image, out);
                }
            }
            Inline::PageNumber
            | Inline::PageCount
            | Inline::Anchor(_)
            | Inline::CommentStart(_)
            | Inline::CommentEnd(_) => {}
        }
        out.push_str("</w:r>");
    }

    /// The character style for a run's non-toggle formatting, one per
    /// (named style, paragraph formatting, run formatting) triple the
    /// document interned; based on the named style when there is one.
    fn formatting_style(
        &mut self,
        paragraph: &Paragraph,
        run: &Run,
        styled: RunProperties,
    ) -> usize {
        let key = (run.style.map_or(0, |id| id as u64 + 1) << 42)
            | (u64::from(paragraph.run_properties.map_or(0, |id| id + 1)) << 21)
            | u64::from(run.properties.map_or(0, |id| id + 1));
        if let Some(index) = self.formatting_styles.get(&key) {
            return *index;
        }
        let index = self.formatting_style_list.len();
        self.formatting_style_list.push((run.style, styled));
        self.formatting_styles.insert(key, index);
        index
    }

    /// A picture, inline or anchored beside the text. Media Word cannot
    /// show as a picture (such as PDF) is left out.
    #[inline(never)]
    fn render_image(&mut self, document: &Document, image: &InlineImage, out: &mut String) {
        let Some(target) = self.media_target(document, image.media) else {
            return;
        };
        let file_name = target.rsplit('/').next().unwrap_or("image").to_string();
        let id = self.relationship_for(RelationshipKind::Image, &target);
        self.drawings += 1;
        let number = self.drawings;
        let width = emu(image.width);
        let height = emu(image.height);
        // A cropped picture: the share cut from each edge, in thousandths of
        // a percent.
        let crop = image
            .crop
            .map_or_else(String::new, |[left, top, right, bottom]| {
                let share = |value: f32| (value * 100_000.0).round() as i64;
                format!(
                    "<a:srcRect l=\"{}\" t=\"{}\" r=\"{}\" b=\"{}\"/>",
                    share(left),
                    share(top),
                    share(right),
                    share(bottom)
                )
            });

        let mut description = String::new();
        escape_attribute(&mut description, image.description.as_deref().unwrap_or(""));
        let mut name = String::new();
        escape_attribute(&mut name, &file_name);
        let extent = format!(
            "<wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>"
        );
        let graphic = format!(
            "<wp:docPr id=\"{number}\" name=\"{name}\" descr=\"{description}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"{PICTURE}\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{number}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rId{id}\"/>{crop}<a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"
        );
        out.push_str("<w:drawing>");
        match image.placement {
            Placement::Inline => {
                let _ = write!(
                    out,
                    "<wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\">{extent}{graphic}</wp:inline>"
                );
            }
            Placement::Floating {
                horizontal,
                vertical,
            } => {
                let relative = |base: AnchorBase| match base {
                    AnchorBase::Page => "page",
                    AnchorBase::Margin => "margin",
                    AnchorBase::Line => "line",
                };
                let _ = write!(
                    out,
                    "<wp:anchor distT=\"0\" distB=\"0\" distL=\"114300\" distR=\"114300\" simplePos=\"0\" relativeHeight=\"{}\" behindDoc=\"0\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"{}\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"{}\"><wp:posOffset>{}</wp:posOffset></wp:positionV>{extent}<wp:wrapSquare wrapText=\"bothSides\"/>{graphic}</wp:anchor>",
                    251_658_240 + number,
                    relative(horizontal.from),
                    emu(horizontal.offset),
                    relative(vertical.from),
                    emu(vertical.offset),
                );
            }
        }
        out.push_str("</w:drawing>");
    }

    /// A floating object as a run holding a page-anchored drawing: an
    /// image, or a text box shape with the blocks inside.
    #[inline(never)]
    fn render_floating(&mut self, document: &Document, index: usize, out: &mut String) {
        let object = &document.floating[index];
        let horizontal = Anchor {
            from: AnchorBase::Page,
            offset: object.x,
        };
        let vertical = Anchor {
            from: if object.follows_text {
                AnchorBase::Line
            } else {
                AnchorBase::Page
            },
            offset: object.y,
        };
        let wrap_xml = match object.wrap {
            TextWrap::Around => "<wp:wrapSquare wrapText=\"bothSides\"/>",
            TextWrap::TopAndBottom => "<wp:wrapTopAndBottom/>",
            TextWrap::None => "<wp:wrapNone/>",
            // Only a shape is written in the line itself (below).
            TextWrap::Inline => "<wp:wrapTopAndBottom/>",
        };
        let behind = u8::from(object.behind);
        // Word measures an anchored object from its paragraph.
        let from_v = if object.follows_text {
            "paragraph"
        } else {
            "page"
        };
        match &object.content {
            FloatingContent::Image(media) => {
                let image = InlineImage {
                    media: *media,
                    width: object.width,
                    height: object.height,
                    description: None,
                    placement: Placement::Floating {
                        horizontal,
                        vertical,
                    },
                    crop: None,
                };
                out.push_str("<w:r>");
                self.render_image(document, &image, out);
                out.push_str("</w:r>");
            }
            FloatingContent::Chart(chart) => {
                self.charts.push(chart_xml(chart));
                let target = format!("charts/chart{}.xml", self.charts.len());
                let relationship = self.relationship_for(RelationshipKind::Chart, &target);
                self.drawings += 1;
                let number = self.drawings;
                let _ = write!(
                    out,
                    "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" simplePos=\"0\" relativeHeight=\"{}\" behindDoc=\"{behind}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"{from_v}\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{}\" cy=\"{}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{wrap_xml}<wp:docPr id=\"{number}\" name=\"Chart {number}\"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri=\"{CHART}\"><c:chart xmlns:c=\"{CHART}\" r:id=\"rId{}\"/></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>",
                    251_658_240 + number,
                    emu(object.x),
                    emu(object.y),
                    emu(object.width),
                    emu(object.height),
                    relationship,
                );
            }
            FloatingContent::TextBox {
                blocks,
                fill,
                line,
                geometry,
                flip,
                ends,
            } => {
                let has_text = !blocks.is_empty();
                let mut content = String::new();
                self.render_blocks(document, blocks, &mut content);
                if content.is_empty() {
                    content.push_str("<w:p/>");
                }
                self.drawings += 1;
                let number = self.drawings;
                let width = emu(object.width);
                let height = emu(object.height);
                let fill_xml = match fill {
                    Some(color) => format!(
                        "<a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>",
                        color.hex()
                    ),
                    None => "<a:noFill/>".to_string(),
                };
                // Arrowheads: Word's head is the line's start, its tail the end.
                let mark = |element: &str, end: Option<crate::document::LineEnd>| {
                    end.map_or_else(String::new, |end| {
                        let kind = match end {
                            crate::document::LineEnd::Arrow => "triangle",
                            crate::document::LineEnd::OpenArrow => "arrow",
                            crate::document::LineEnd::Diamond => "diamond",
                            crate::document::LineEnd::Circle => "oval",
                        };
                        format!("<a:{element} type=\"{kind}\"/>")
                    })
                };
                let line_xml = match line {
                    Some(line) => format!(
                        "<a:ln w=\"{}\"><a:solidFill><a:srgbClr val=\"{}\"/></a:solidFill>{}{}</a:ln>",
                        emu(line.width),
                        line.color
                            .map_or_else(|| "000000".to_string(), |color| color.hex()),
                        mark("headEnd", ends.0),
                        mark("tailEnd", ends.1)
                    ),
                    None => "<a:ln><a:noFill/></a:ln>".to_string(),
                };
                let geometry_xml = geometry_xml(document, *geometry);
                let flips = format!(
                    "{}{}",
                    if flip.0 { " flipH=\"1\"" } else { "" },
                    if flip.1 { " flipV=\"1\"" } else { "" }
                );
                // A shape with text holds it in a text box body; one without
                // is only drawn.
                let (kind, body) = if has_text {
                    (
                        "<wps:cNvSpPr txBox=\"1\"/>",
                        format!("<wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx>"),
                    )
                } else {
                    ("<wps:cNvSpPr/>", String::new())
                };
                // A shape in the text line sits in it; any other is anchored.
                let (frame, frame_end) = if object.wrap == TextWrap::Inline {
                    (
                        format!(
                            "<wp:inline distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\"><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>"
                        ),
                        "</wp:inline>",
                    )
                } else {
                    (
                        format!(
                            "<wp:anchor distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" simplePos=\"0\" relativeHeight=\"{}\" behindDoc=\"{behind}\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"{from_v}\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>{wrap_xml}",
                            251_658_240 + number,
                            emu(object.x),
                            emu(object.y),
                        ),
                        "</wp:anchor>",
                    )
                };
                let _ = write!(
                    out,
                    "<w:r><w:drawing>{frame}<wp:docPr id=\"{number}\" name=\"Shape {number}\"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri=\"{SHAPE}\"><wps:wsp>{kind}<wps:spPr><a:xfrm{flips}><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm>{geometry_xml}{fill_xml}{line_xml}</wps:spPr>{body}<wps:bodyPr wrap=\"square\" lIns=\"50800\" tIns=\"50800\" rIns=\"50800\" bIns=\"50800\" anchor=\"t\"><a:noAutofit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic>{frame_end}</w:drawing></w:r>",
                );
            }
        }
    }

    /// The package path of a media file under `word/`, when Word can show
    /// it as a picture.
    #[inline(never)]
    fn media_target(&mut self, document: &Document, media: usize) -> Option<String> {
        if let Some(Some(target)) = self.media_targets.get(media) {
            return Some(target.clone());
        }
        let file = document.media.get(media)?;
        let extension = file
            .name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        image_content_type(&extension)?;
        let target = format!("media/image{}.{extension}", media + 1);
        if self.media_targets.len() <= media {
            self.media_targets.resize(media + 1, None);
        }
        self.media_targets[media] = Some(target.clone());
        Some(target)
    }

    /// Whether the document needs a settings part.
    /// Every package has settings, as Word's own do: LibreOffice lays out a
    /// package without them by older rules (wider gaps between paragraphs),
    /// so a page can run long.
    fn needs_settings(&self, _document: &Document) -> bool {
        true
    }

    fn relationship_for(&mut self, kind: RelationshipKind, target: &str) -> usize {
        self.relationships.push(Relationship {
            kind,
            target: target.to_string(),
        });
        self.relationships.len() - 1 + 10
    }

    /// Renders blocks as a part of their own, with their own relationships.
    fn render_part(
        &mut self,
        document: &Document,
        blocks: &[Block],
    ) -> (String, Vec<Relationship>) {
        let outer = std::mem::take(&mut self.relationships);
        let mut xml = String::new();
        self.render_blocks(document, blocks, &mut xml);
        let relationships = std::mem::replace(&mut self.relationships, outer);
        (xml, relationships)
    }

    /// A header or footer part; the relationship id the section refers
    /// to it by.
    #[inline(never)]
    fn page_part(
        &mut self,
        document: &Document,
        blocks: &[Block],
        kind: RelationshipKind,
        part: crate::document::PagePart,
    ) -> usize {
        let (mut inner, relationships) = self.render_part(document, blocks);
        // The drawings this header or footer repeats on its pages, anchored
        // in a paragraph of their own at its start (with the part's own
        // relationships, which they add to).
        let outer = std::mem::replace(&mut self.relationships, relationships);
        let mut drawings = String::new();
        for index in 0..document.floating.len() {
            if !self.floating_done[index] && document.floating[index].repeats == Some(part) {
                self.floating_done[index] = true;
                self.render_floating(document, index, &mut drawings);
            }
        }
        let relationships = std::mem::replace(&mut self.relationships, outer);
        if !drawings.is_empty() {
            inner.insert_str(0, &format!("<w:p>{drawings}</w:p>"));
        }
        let (element, prefix) = match kind {
            RelationshipKind::Header => ("w:hdr", "header"),
            _ => ("w:ftr", "footer"),
        };
        let name = format!("{prefix}{}.xml", self.page_parts.len() + 1);
        let mut xml = String::with_capacity(inner.len() + 512);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<{element} {W} {R} {DRAWING}>");
        if inner.is_empty() {
            xml.push_str("<w:p/>");
        } else {
            xml.push_str(&inner);
        }
        let _ = write!(xml, "</{element}>");
        self.page_parts.push(Part {
            name: name.clone(),
            xml,
            relationships,
        });
        self.relationship_for(kind, &name)
    }

    #[inline(never)]
    fn section_properties(&mut self, document: &Document, section: &Section) -> String {
        let page = &section.page;
        let mut xml = String::new();
        xml.push_str("<w:sectPr>");
        let variants = [
            (
                &section.headers,
                RelationshipKind::Header,
                "headerReference",
            ),
            (
                &section.footers,
                RelationshipKind::Footer,
                "footerReference",
            ),
        ];
        for (variant, kind, element) in variants {
            let pages = [
                ("default", &variant.default, PageKind::Default),
                ("first", &variant.first, PageKind::First),
                ("even", &variant.even, PageKind::Even),
            ];
            for (page_kind, blocks, pages) in pages {
                let Some(blocks) = blocks else {
                    continue;
                };
                let part = crate::document::PagePart {
                    footer: kind != RelationshipKind::Header,
                    pages,
                };
                let id = self.page_part(document, blocks, kind, part);
                let _ = write!(
                    xml,
                    "<w:{element} w:type=\"{page_kind}\" r:id=\"rId{id}\"/>"
                );
            }
        }
        if section.start == SectionStart::Continuous {
            xml.push_str("<w:type w:val=\"continuous\"/>");
        }
        let _ = write!(
            xml,
            "<w:pgSz w:w=\"{}\" w:h=\"{}\"/>",
            twips(page.width),
            twips(page.height)
        );
        let _ = write!(
            xml,
            "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"{}\" w:footer=\"{}\" w:gutter=\"0\"/>",
            twips(page.margin_top),
            twips(page.margin_right),
            twips(page.margin_bottom),
            twips(page.margin_left),
            twips(page.header_distance),
            twips(page.footer_distance),
        );
        if let Some(start) = page.page_number_start {
            let _ = write!(xml, "<w:pgNumType w:start=\"{start}\"/>");
        }
        if section.columns > 1 {
            let gap = section.column_gap.map_or(708, twips);
            if section.column_widths.is_empty() {
                let _ = write!(
                    xml,
                    "<w:cols w:num=\"{}\" w:space=\"{gap}\"/>",
                    section.columns
                );
            } else {
                let _ = write!(
                    xml,
                    "<w:cols w:num=\"{}\" w:space=\"{gap}\" w:equalWidth=\"0\">",
                    section.columns
                );
                for (width, space) in &section.column_widths {
                    let _ = write!(
                        xml,
                        "<w:col w:w=\"{}\" w:space=\"{}\"/>",
                        twips(*width),
                        twips(*space)
                    );
                }
                xml.push_str("</w:cols>");
            }
        }
        if section.headers.first.is_some() || section.footers.first.is_some() {
            xml.push_str("<w:titlePg/>");
        }
        if section.headers.even.is_some() || section.footers.even.is_some() {
            self.even_pages = true;
        }
        xml.push_str("</w:sectPr>");
        xml
    }

    /// The relationships of a part; the document part also links the
    /// styles, numbering, footnotes, and settings parts.
    fn relationships_xml(
        &self,
        document: &Document,
        relationships: &[Relationship],
        is_document: bool,
    ) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<Relationships xmlns=\"{PACKAGE_REL}\">");
        if is_document {
            let _ = write!(
                xml,
                "<Relationship Id=\"rId1\" Type=\"{REL}/styles\" Target=\"styles.xml\"/>"
            );
            if !self.numbering.instances.is_empty() {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId2\" Type=\"{REL}/numbering\" Target=\"numbering.xml\"/>"
                );
            }
            if !document.footnotes.is_empty() {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId3\" Type=\"{REL}/footnotes\" Target=\"footnotes.xml\"/>"
                );
            }
            if self.needs_settings(document) {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId4\" Type=\"{REL}/settings\" Target=\"settings.xml\"/>"
                );
            }
            if !document.comments.is_empty() {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId5\" Type=\"{REL}/comments\" Target=\"comments.xml\"/><Relationship Id=\"rId6\" Type=\"http://schemas.microsoft.com/office/2011/relationships/commentsExtended\" Target=\"commentsExtended.xml\"/>"
                );
            }
        }
        for (index, relationship) in relationships.iter().enumerate() {
            let kind = match relationship.kind {
                RelationshipKind::Image => "image",
                RelationshipKind::Header => "header",
                RelationshipKind::Footer => "footer",
                RelationshipKind::Chart => "chart",
            };
            let _ = write!(
                xml,
                "<Relationship Id=\"rId{}\" Type=\"{REL}/{kind}\" Target=\"",
                index + 10
            );
            escape_attribute(&mut xml, &relationship.target);
            xml.push_str("\"/>");
        }
        xml.push_str("</Relationships>");
        xml
    }

    fn content_types(&self, document: &Document) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        xml.push_str(
            "<Types xmlns=\"http://schemas.openxmlformats.org/package/2006/content-types\">",
        );
        xml.push_str("<Default Extension=\"rels\" ContentType=\"application/vnd.openxmlformats-package.relationships+xml\"/>");
        xml.push_str("<Default Extension=\"xml\" ContentType=\"application/xml\"/>");
        let mut extensions: Vec<&str> = Vec::new();
        for target in self.media_targets.iter().flatten() {
            let extension = target.rsplit('.').next().unwrap_or("");
            if !extensions.contains(&extension) {
                extensions.push(extension);
            }
        }
        for extension in extensions {
            if let Some(content_type) = image_content_type(extension) {
                let _ = write!(
                    xml,
                    "<Default Extension=\"{extension}\" ContentType=\"{content_type}\"/>"
                );
            }
        }
        xml.push_str("<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>");
        xml.push_str("<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>");
        if !self.numbering.instances.is_empty() {
            xml.push_str("<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>");
        }
        if !document.footnotes.is_empty() {
            xml.push_str("<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>");
        }
        if self.needs_settings(document) {
            xml.push_str("<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>");
        }
        if !document.comments.is_empty() {
            xml.push_str("<Override PartName=\"/word/comments.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.comments+xml\"/>");
            xml.push_str("<Override PartName=\"/word/commentsExtended.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.commentsExtended+xml\"/>");
        }
        for index in 0..self.charts.len() {
            let _ = write!(
                xml,
                "<Override PartName=\"/word/charts/chart{}.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.drawingml.chart+xml\"/>",
                index + 1
            );
        }
        for part in &self.page_parts {
            let kind = if part.name.starts_with("header") {
                "header"
            } else {
                "footer"
            };
            let _ = write!(
                xml,
                "<Override PartName=\"/word/{}\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.{kind}+xml\"/>",
                part.name
            );
        }
        xml.push_str("</Types>");
        xml
    }

    fn styles_xml(&self, document: &Document) -> String {
        let styles = &document.styles;
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:styles {W}>");
        xml.push_str("<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults>");
        // The document's own default paragraph style, when it has one, is
        // Word's default; otherwise an empty Normal is.
        if styles.default_paragraph.is_none() {
            xml.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>");
        }
        xml.push_str("<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/></w:style>");
        xml.push_str("<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>");
        xml.push_str("<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:left w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:right w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/></w:tblBorders><w:tblCellMar><w:top w:w=\"80\" w:type=\"dxa\"/><w:left w:w=\"80\" w:type=\"dxa\"/><w:bottom w:w=\"80\" w:type=\"dxa\"/><w:right w:w=\"80\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>");
        for (index, style) in styles.paragraph.iter().enumerate() {
            let default = if styles.default_paragraph == Some(index) {
                " w:default=\"1\""
            } else {
                ""
            };
            let _ = write!(
                xml,
                "<w:style w:type=\"paragraph\"{default} w:styleId=\"{}\"><w:name w:val=\"",
                paragraph_style_id(document, index)
            );
            escape_attribute(&mut xml, &style.name);
            xml.push_str("\"/>");
            if let Some(parent) = style.parent {
                let _ = write!(
                    xml,
                    "<w:basedOn w:val=\"{}\"/>",
                    paragraph_style_id(document, parent)
                );
            }
            let mut paragraph = String::new();
            paragraph_properties_xml(
                &style.paragraph,
                None,
                document.tab_set(style.paragraph.tabs),
                &mut paragraph,
            );
            if !paragraph.is_empty() {
                let _ = write!(xml, "<w:pPr>{paragraph}</w:pPr>");
            }
            let mut run = String::new();
            run_properties_xml(document, &style.run, &mut run);
            if !run.is_empty() {
                let _ = write!(xml, "<w:rPr>{run}</w:rPr>");
            }
            xml.push_str("</w:style>");
        }
        for style in &styles.character {
            let _ = write!(
                xml,
                "<w:style w:type=\"character\" w:styleId=\"{}\"><w:name w:val=\"",
                style_id(&style.name)
            );
            escape_attribute(&mut xml, &style.name);
            xml.push_str("\"/>");
            if let Some(parent) = style.parent {
                let _ = write!(
                    xml,
                    "<w:basedOn w:val=\"{}\"/>",
                    style_id(&styles.character[parent].name)
                );
            }
            let mut run = String::new();
            run_properties_xml(document, &style.run, &mut run);
            if !run.is_empty() {
                let _ = write!(xml, "<w:rPr>{run}</w:rPr>");
            }
            xml.push_str("</w:style>");
        }
        for (index, (based_on, formatting)) in self.formatting_style_list.iter().enumerate() {
            let _ = write!(
                xml,
                "<w:style w:type=\"character\" w:styleId=\"r{index}\"><w:name w:val=\"Run {index}\"/>"
            );
            if let Some(parent) = based_on {
                let _ = write!(
                    xml,
                    "<w:basedOn w:val=\"{}\"/>",
                    style_id(&styles.character[*parent].name)
                );
            }
            xml.push_str("<w:rPr>");
            run_properties_xml(document, formatting, &mut xml);
            xml.push_str("</w:rPr></w:style>");
        }
        xml.push_str("</w:styles>");
        xml
    }

    /// Writes `numbering.xml` in parts: the list styles, then one
    /// `w:num` per list start, of which a long document has many.
    fn write_numbering<W: io::Write>(
        &self,
        document: &Document,
        zip: &mut ZipWriter<W>,
    ) -> io::Result<()> {
        zip.begin_deflated("word/numbering.xml")?;
        let mut xml = self.numbering_styles_xml(document);
        for (index, (style, start)) in self.numbering.instances.iter().enumerate() {
            let _ = write!(
                xml,
                "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{style}\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"{start}\"/></w:lvlOverride></w:num>",
                index + 1
            );
            if xml.len() >= BODY_PART {
                zip.write_part(xml.as_bytes(), Level::Default)?;
                xml.clear();
            }
        }
        xml.push_str("</w:numbering>");
        zip.write_part(xml.as_bytes(), Level::Default)?;
        zip.end_deflated()
    }

    /// The head of `numbering.xml`: the abstract numbering of each list
    /// style.
    fn numbering_styles_xml(&self, document: &Document) -> String {
        let styles = &document.styles.list;
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:numbering {W}>");
        for (index, style) in styles.iter().enumerate() {
            let _ = write!(
                xml,
                "<w:abstractNum w:abstractNumId=\"{index}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"
            );
            for (level, definition) in style.levels.iter().enumerate().take(9) {
                let mut font = None;
                let (format, text) = match &definition.label {
                    ListLabel::None => ("none", String::new()),
                    ListLabel::Text(marker) => {
                        // Bullets in the symbol fonts Word itself uses: a
                        // marker in no font is drawn in a fallback font,
                        // whose taller lines lengthen every list item.
                        let (symbol, marker) = word_bullet(marker);
                        font = symbol;
                        ("bullet", marker)
                    }
                    ListLabel::Number(number) => (
                        match number.kind {
                            NumberKind::Decimal => "decimal",
                            NumberKind::LowerLetter => "lowerLetter",
                            NumberKind::UpperLetter => "upperLetter",
                            NumberKind::LowerRoman => "lowerRoman",
                            NumberKind::UpperRoman => "upperRoman",
                        },
                        {
                            // A tiered level leads with its parents' numbers.
                            let parents: String = if number.tiered {
                                (1..=level).map(|parent| format!("%{parent}.")).collect()
                            } else {
                                String::new()
                            };
                            parents + &number.pattern.replace("%1", &format!("%{}", level + 1))
                        },
                    ),
                };
                let left = twips(definition.indent);
                let hanging = twips(definition.indent - definition.label_indent).max(0);
                let _ = write!(
                    xml,
                    "<w:lvl w:ilvl=\"{level}\"><w:start w:val=\"1\"/><w:numFmt w:val=\"{format}\"/><w:lvlText w:val=\""
                );
                escape_attribute(&mut xml, &text);
                let _ = write!(
                    xml,
                    "\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"{hanging}\"/></w:pPr>"
                );
                if let Some(font) = font {
                    let _ = write!(
                        xml,
                        "<w:rPr><w:rFonts w:ascii=\"{font}\" w:hAnsi=\"{font}\" w:hint=\"default\"/></w:rPr>"
                    );
                }
                xml.push_str("</w:lvl>");
            }
            xml.push_str("</w:abstractNum>");
        }
        xml
    }

    fn footnotes_part(&mut self, document: &Document) -> Part {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:footnotes {W} {R} {DRAWING}>");
        xml.push_str("<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>");
        xml.push_str("<w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
        let outer = std::mem::take(&mut self.relationships);
        for (index, note) in document.footnotes.iter().enumerate() {
            let _ = write!(xml, "<w:footnote w:id=\"{}\">", index + 1);
            let mut body = String::new();
            self.render_blocks(document, &note.blocks, &mut body);
            // The reference mark leads the first paragraph.
            match body.find("<w:p>") {
                Some(_) => {
                    let mark = "<w:r><w:rPr><w:vertAlign w:val=\"superscript\"/></w:rPr><w:footnoteRef/></w:r><w:r><w:t xml:space=\"preserve\"> </w:t></w:r>";
                    let insert_at = body.find("</w:pPr>").map_or(5, |end| end + 8);
                    body.insert_str(insert_at, mark);
                }
                None => body.push_str("<w:p/>"),
            }
            xml.push_str(&body);
            xml.push_str("</w:footnote>");
        }
        xml.push_str("</w:footnotes>");
        let relationships = std::mem::replace(&mut self.relationships, outer);
        Part {
            name: "footnotes.xml".to_string(),
            xml,
            relationships,
        }
    }
}

impl Numbering {
    /// The `w:num` a paragraph uses; a new one per list start.
    fn instance_for(&mut self, style: usize, starts_list: bool, start: u32) -> usize {
        if self.current.len() <= style {
            self.current.resize(style + 1, None);
        }
        match self.current[style] {
            Some(instance) if !starts_list => instance,
            _ => {
                self.instances.push((style, start));
                let instance = self.instances.len() - 1;
                self.current[style] = Some(instance);
                instance
            }
        }
    }
}

fn root_relationships() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><Relationships xmlns=\"{PACKAGE_REL}\"><Relationship Id=\"rId1\" Type=\"{REL}/officeDocument\" Target=\"word/document.xml\"/></Relationships>"
    )
}

/// The settings part: odd and even pages' own headers, and showing the page
/// colour (Word draws it only when told to).
fn settings_xml(even_pages: bool, page_color: bool) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:settings {W}>{}{}<w:compat><w:compatSetting w:name=\"compatibilityMode\" w:uri=\"http://schemas.microsoft.com/office/word\" w:val=\"15\"/></w:compat></w:settings>",
        if page_color {
            "<w:displayBackgroundShape/>"
        } else {
            ""
        },
        if even_pages {
            "<w:evenAndOddHeaders/>"
        } else {
            ""
        }
    )
}

/// The content type of an image file Word can show, by extension.
fn image_content_type(extension: &str) -> Option<&'static str> {
    match extension {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        "tif" | "tiff" => Some("image/tiff"),
        _ => None,
    }
}

/// A paragraph's properties in `w:pPr` schema order, with its list
/// numbering (`w:numPr`) and tab stops placed where Word expects them.
fn paragraph_properties_xml(
    properties: &ParagraphProperties,
    numbering: Option<&str>,
    tabs: &[crate::document::TabStop],
    out: &mut String,
) {
    if properties.keep_with_next == Some(true) {
        out.push_str("<w:keepNext/>");
    }
    if properties.keep_lines_together == Some(true) {
        out.push_str("<w:keepLines/>");
    }
    if let Some(page_break) = properties.page_break_before {
        out.push_str(if page_break {
            "<w:pageBreakBefore/>"
        } else {
            "<w:pageBreakBefore w:val=\"0\"/>"
        });
    }
    if let Some(widows) = properties.widow_control {
        let _ = write!(
            out,
            "<w:widowControl w:val=\"{}\"/>",
            if widows { "1" } else { "0" }
        );
    }
    if let Some(numbering) = numbering {
        out.push_str(numbering);
    }
    if let Some(border) = properties.border {
        out.push_str("<w:pBdr>");
        for (name, drawn) in [
            ("top", border.top),
            ("left", border.left),
            ("bottom", border.bottom),
            ("right", border.right),
        ] {
            if drawn {
                border_xml(name, Some(Some(border.line)), out);
            }
        }
        out.push_str("</w:pBdr>");
    }
    if let Some(background) = properties.background {
        let _ = write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            background.hex()
        );
    }
    if !tabs.is_empty() {
        out.push_str("<w:tabs>");
        for tab in tabs {
            let alignment = match tab.alignment {
                crate::document::TabAlignment::Left => "left",
                crate::document::TabAlignment::Center => "center",
                crate::document::TabAlignment::Right => "right",
                crate::document::TabAlignment::Decimal => "decimal",
            };
            let leader = match tab.leader {
                Some('.') => " w:leader=\"dot\"",
                Some('-') => " w:leader=\"hyphen\"",
                Some('_') => " w:leader=\"underscore\"",
                Some('·') => " w:leader=\"middleDot\"",
                _ => "",
            };
            let _ = write!(
                out,
                "<w:tab w:val=\"{alignment}\"{leader} w:pos=\"{}\"/>",
                twips(tab.position)
            );
        }
        out.push_str("</w:tabs>");
    }
    let has_spacing = properties.space_before.is_some()
        || properties.space_after.is_some()
        || properties.line_spacing.is_some();
    if has_spacing {
        out.push_str("<w:spacing");
        if let Some(before) = properties.space_before {
            let _ = write!(out, " w:before=\"{}\"", twips(before));
        }
        if let Some(after) = properties.space_after {
            let _ = write!(out, " w:after=\"{}\"", twips(after));
        }
        match properties.line_spacing {
            Some(LineSpacing::Relative(multiple)) => {
                let _ = write!(
                    out,
                    " w:line=\"{}\" w:lineRule=\"auto\"",
                    (multiple * 240.0).round() as i64
                );
            }
            Some(LineSpacing::Minimum(points)) => {
                let _ = write!(out, " w:line=\"{}\" w:lineRule=\"atLeast\"", twips(points));
            }
            Some(LineSpacing::Exact(points)) => {
                let _ = write!(out, " w:line=\"{}\" w:lineRule=\"exact\"", twips(points));
            }
            None => {}
        }
        out.push_str("/>");
    }
    let has_indent = properties.left_indent.is_some()
        || properties.right_indent.is_some()
        || properties.first_line_indent.is_some();
    if has_indent {
        out.push_str("<w:ind");
        let left = properties.left_indent.unwrap_or(0.0);
        if properties.left_indent.is_some() {
            let _ = write!(out, " w:left=\"{}\"", twips(left));
        }
        if let Some(right) = properties.right_indent {
            let _ = write!(out, " w:right=\"{}\"", twips(right));
        }
        // Pages measures the first line from the margin; Word from the left indent.
        if let Some(first) = properties.first_line_indent {
            let relative = first - left;
            if relative < 0.0 {
                let _ = write!(out, " w:hanging=\"{}\"", twips(-relative));
            } else {
                let _ = write!(out, " w:firstLine=\"{}\"", twips(relative));
            }
        }
        out.push_str("/>");
    }
    if let Some(alignment) = properties.alignment {
        let value = match alignment {
            Alignment::Left => "left",
            Alignment::Center => "center",
            Alignment::Right => "right",
            Alignment::Justify => "both",
        };
        let _ = write!(out, "<w:jc w:val=\"{value}\"/>");
    }
    if let Some(level) = properties.outline_level {
        let _ = write!(out, "<w:outlineLvl w:val=\"{level}\"/>");
    }
}

fn run_properties_xml(document: &Document, properties: &RunProperties, out: &mut String) {
    if let Some(font) = properties.font {
        let family = font_family(document.string(font));
        out.push_str("<w:rFonts w:ascii=\"");
        escape_attribute(out, &family);
        out.push_str("\" w:hAnsi=\"");
        escape_attribute(out, &family);
        out.push_str("\" w:cs=\"");
        escape_attribute(out, &family);
        out.push_str("\"/>");
    }
    if let Some(bold) = properties.bold {
        out.push_str(if bold { "<w:b/>" } else { "<w:b w:val=\"0\"/>" });
    }
    if let Some(italic) = properties.italic {
        out.push_str(if italic {
            "<w:i/>"
        } else {
            "<w:i w:val=\"0\"/>"
        });
    }
    match properties.caps {
        Some(Caps::All) => out.push_str("<w:caps/>"),
        Some(Caps::Small) => out.push_str("<w:smallCaps/>"),
        None => {}
    }
    if let Some(strike) = properties.strike {
        out.push_str(if strike {
            "<w:strike/>"
        } else {
            "<w:strike w:val=\"0\"/>"
        });
    }
    if let Some(hidden) = properties.hidden {
        out.push_str(if hidden {
            "<w:vanish/>"
        } else {
            "<w:vanish w:val=\"0\"/>"
        });
    }
    if let Some(color) = properties.color {
        let _ = write!(out, "<w:color w:val=\"{}\"/>", color.hex());
    }
    if let Some(spacing) = properties.letter_spacing {
        let _ = write!(
            out,
            "<w:spacing w:val=\"{}\"/>",
            (spacing * 20.0).round() as i64
        );
    }
    if let Some(scale) = properties.width_scale {
        let _ = write!(out, "<w:w w:val=\"{}\"/>", scale.round() as i64);
    }
    if let Some(shift) = properties.shift {
        let _ = write!(
            out,
            "<w:position w:val=\"{}\"/>",
            (shift * 2.0).round() as i64
        );
    }
    if let Some(size) = properties.size {
        let half_points = (size * 2.0).round() as i64;
        let _ = write!(out, "<w:sz w:val=\"{half_points}\"/>");
    }
    // Schema order: the underline comes before the shading.
    if let Some(underline) = properties.underline {
        out.push_str(if underline {
            "<w:u w:val=\"single\"/>"
        } else {
            "<w:u w:val=\"none\"/>"
        });
    }
    if let Some(highlight) = properties.highlight {
        let _ = write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            highlight.hex()
        );
    }
    match properties.baseline {
        Some(Baseline::Superscript) => out.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        Some(Baseline::Subscript) => out.push_str("<w:vertAlign w:val=\"subscript\"/>"),
        None => {}
    }
    if let Some(language) = properties.language {
        out.push_str("<w:lang w:val=\"");
        escape_attribute(out, document.string(language));
        out.push_str("\"/>");
    }
}

/// Splits run formatting into what a character style can carry without
/// changing meaning (fonts, size, colors, underline, baseline, language)
/// and the toggle properties that must stay inline.
fn split_for_style(merged: RunProperties) -> (RunProperties, RunProperties) {
    let styled = RunProperties {
        font: merged.font,
        size: merged.size,
        color: merged.color,
        highlight: merged.highlight,
        underline: merged.underline,
        baseline: merged.baseline,
        language: merged.language,
        shift: merged.shift,
        ..RunProperties::default()
    };
    let inline = RunProperties {
        bold: merged.bold,
        italic: merged.italic,
        strike: merged.strike,
        caps: merged.caps,
        hidden: merged.hidden,
        ..RunProperties::default()
    };
    (styled, inline)
}

/// Pages names fonts by PostScript name (`HelveticaNeue-Bold`,
/// `ComicSansMS`, `TimesNewRomanPSMT`); Word wants the family as the
/// user knows it. The style after the hyphen and the `PSMT`/`MT` tails are
/// dropped, and words run together are split at their capitals.
fn font_family(postscript_name: &str) -> String {
    if postscript_name.contains(' ') {
        return postscript_name.to_string();
    }
    let base = postscript_name.split('-').next().unwrap_or(postscript_name);
    let base = base
        .strip_suffix("PSMT")
        .or_else(|| base.strip_suffix("MT"))
        .unwrap_or(base);
    let mut family = String::with_capacity(base.len() + 4);
    let mut previous: Option<char> = None;
    let characters: Vec<char> = base.chars().collect();
    for (index, ch) in characters.iter().enumerate() {
        if let Some(before) = previous {
            let next_is_lower = characters.get(index + 1).is_some_and(|c| c.is_lowercase());
            let starts_word = ch.is_uppercase()
                && (before.is_lowercase() || (before.is_uppercase() && next_is_lower));
            let starts_number = ch.is_ascii_digit() && !before.is_ascii_digit();
            if starts_word || starts_number {
                family.push(' ');
            }
        }
        family.push(*ch);
        previous = Some(*ch);
    }
    family
}

/// The style identifier: the name itself, as Pages exports it (Word
/// accepts spaces), with only the characters an attribute cannot hold
/// removed.
/// A paragraph style's id: `Normal` for the document's default style, and
/// never `Normal` for another (which would shadow the default).
fn paragraph_style_id(document: &Document, index: usize) -> String {
    if document.styles.default_paragraph == Some(index) {
        return "Normal".to_string();
    }
    let id = style_id(&document.styles.paragraph[index].name);
    if id == "Normal" {
        format!("Normal{}", index + 1)
    } else {
        id
    }
}

fn style_id(name: &str) -> String {
    let mut id: String = name
        .chars()
        .filter(|ch| !matches!(ch, '"' | '<' | '>' | '&'))
        .collect();
    if id.is_empty() {
        id.push_str("Style");
    }
    id
}

fn twips(points: f32) -> i64 {
    (points * 20.0).round() as i64
}

/// English metric units, as drawings are measured.
fn emu(points: f32) -> i64 {
    (points * 12_700.0).round() as i64
}

/// One border side: a line, `nil` for a stated "no line", or nothing when
/// unstated.
fn border_xml(name: &str, side: Option<Option<crate::document::Border>>, out: &mut String) {
    match side {
        Some(Some(line)) => {
            let color = line
                .color
                .map_or_else(|| "auto".to_string(), |color| color.hex());
            let _ = write!(
                out,
                "<w:{name} w:val=\"single\" w:sz=\"{}\" w:space=\"0\" w:color=\"{color}\"/>",
                ((line.width * 8.0).round() as i64).clamp(2, 96)
            );
        }
        Some(None) => {
            let _ = write!(out, "<w:{name} w:val=\"nil\"/>");
        }
        None => {}
    }
}

/// The DrawingML chart namespace (also the chart graphic's URI).
const CHART: &str = "http://schemas.openxmlformats.org/drawingml/2006/chart";

/// A chart part: the chart's kind, its series, and their cached values
/// (which Word draws from; there is no embedded workbook).
fn chart_xml(chart: &crate::document::Chart) -> String {
    use crate::document::ChartKind;
    let mut xml = String::new();
    let _ = write!(
        xml,
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><c:chartSpace xmlns:c=\"{CHART}\" xmlns:a=\"http://schemas.openxmlformats.org/drawingml/2006/main\" xmlns:r=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships\"><c:roundedCorners val=\"0\"/><c:chart><c:autoTitleDeleted val=\"1\"/><c:plotArea><c:layout/>"
    );
    let (element, extra) = match chart.kind {
        ChartKind::Column => (
            "c:barChart",
            "<c:barDir val=\"col\"/><c:grouping val=\"clustered\"/>",
        ),
        ChartKind::Bar => (
            "c:barChart",
            "<c:barDir val=\"bar\"/><c:grouping val=\"clustered\"/>",
        ),
        ChartKind::Line => ("c:lineChart", "<c:grouping val=\"standard\"/>"),
        ChartKind::Area => ("c:areaChart", "<c:grouping val=\"standard\"/>"),
        ChartKind::Pie => ("c:pieChart", ""),
        ChartKind::Scatter => ("c:scatterChart", "<c:scatterStyle val=\"lineMarker\"/>"),
    };
    let _ = write!(xml, "<{element}>{extra}<c:varyColors val=\"0\"/>");
    let count = chart.categories.len();
    for (index, series) in chart.series.iter().enumerate() {
        let _ = write!(
            xml,
            "<c:ser><c:idx val=\"{index}\"/><c:order val=\"{index}\"/><c:tx><c:strRef><c:f/><c:strCache><c:ptCount val=\"1\"/><c:pt idx=\"0\"><c:v>"
        );
        escape_text(&mut xml, &series.name);
        xml.push_str("</c:v></c:pt></c:strCache></c:strRef></c:tx>");
        // Each series in its own colour, as Pages draws them.
        const PALETTE: [&str; 6] = ["4A9CF5", "7ED957", "F5A623", "E8505B", "9B6BD9", "4FC9C1"];
        let color = PALETTE[index % PALETTE.len()];
        if chart.kind == ChartKind::Line || chart.kind == ChartKind::Scatter {
            let _ = write!(
                xml,
                "<c:spPr><a:ln w=\"28575\"><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></a:ln></c:spPr>"
            );
        } else {
            let _ = write!(
                xml,
                "<c:spPr><a:solidFill><a:srgbClr val=\"{color}\"/></a:solidFill></c:spPr>"
            );
        }
        let (categories, values) = if chart.kind == ChartKind::Scatter {
            ("c:xVal", "c:yVal")
        } else {
            ("c:cat", "c:val")
        };
        let _ = write!(
            xml,
            "<{categories}><c:strRef><c:f/><c:strCache><c:ptCount val=\"{count}\"/>"
        );
        for (point, category) in chart.categories.iter().enumerate() {
            let _ = write!(xml, "<c:pt idx=\"{point}\"><c:v>");
            escape_text(&mut xml, category);
            xml.push_str("</c:v></c:pt>");
        }
        let _ = write!(
            xml,
            "</c:strCache></c:strRef></{categories}><{values}><c:numRef><c:f/><c:numCache><c:formatCode>General</c:formatCode><c:ptCount val=\"{count}\"/>"
        );
        for (point, value) in series.values.iter().enumerate() {
            if let Some(value) = value {
                let _ = write!(xml, "<c:pt idx=\"{point}\"><c:v>{value}</c:v></c:pt>");
            }
        }
        let _ = write!(xml, "</c:numCache></c:numRef></{values}></c:ser>");
    }
    if chart.kind == ChartKind::Pie {
        let _ = write!(xml, "</{element}>");
    } else {
        let _ = write!(
            xml,
            "<c:axId val=\"1\"/><c:axId val=\"2\"/></{element}><c:catAx><c:axId val=\"1\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{}\"/><c:crossAx val=\"2\"/></c:catAx><c:valAx><c:axId val=\"2\"/><c:scaling><c:orientation val=\"minMax\"/></c:scaling><c:delete val=\"0\"/><c:axPos val=\"{}\"/><c:majorGridlines/><c:crossAx val=\"1\"/></c:valAx>",
            if chart.kind == ChartKind::Bar {
                "l"
            } else {
                "b"
            },
            if chart.kind == ChartKind::Bar {
                "b"
            } else {
                "l"
            },
        );
    }
    xml.push_str("</c:plotArea><c:legend><c:legendPos val=\"t\"/><c:overlay val=\"0\"/></c:legend><c:plotVisOnly val=\"1\"/></c:chart></c:chartSpace>");
    xml
}

/// A shape's outline in DrawingML: a preset by name, or its own path.
fn geometry_xml(document: &Document, geometry: crate::document::ShapeGeometry) -> String {
    use crate::document::{PathStep, ShapeGeometry as G};
    let preset = match geometry {
        G::Rectangle => "rect",
        G::RoundedRectangle => "roundRect",
        G::Ellipse => "ellipse",
        G::Triangle => "triangle",
        G::RightTriangle => "rtTriangle",
        G::Diamond => "diamond",
        G::Pentagon => "pentagon",
        G::Hexagon => "hexagon",
        G::Octagon => "octagon",
        G::Star => "star5",
        G::RightArrow => "rightArrow",
        G::LeftArrow => "leftArrow",
        G::UpArrow => "upArrow",
        G::DownArrow => "downArrow",
        G::Line => "line",
        G::Path(index) => {
            let Some(path) = document.paths.get(index as usize) else {
                return "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>".to_string();
            };
            let point = |(x, y): (f32, f32)| format!("<a:pt x=\"{}\" y=\"{}\"/>", emu(x), emu(y));
            let mut commands = String::new();
            for step in &path.steps {
                match *step {
                    PathStep::Move(x, y) => {
                        let _ = write!(commands, "<a:moveTo>{}</a:moveTo>", point((x, y)));
                    }
                    PathStep::Line(x, y) => {
                        let _ = write!(commands, "<a:lnTo>{}</a:lnTo>", point((x, y)));
                    }
                    PathStep::Curve(points) => {
                        commands.push_str("<a:cubicBezTo>");
                        for p in points {
                            commands.push_str(&point(p));
                        }
                        commands.push_str("</a:cubicBezTo>");
                    }
                    PathStep::Close => commands.push_str("<a:close/>"),
                }
            }
            return format!(
                "<a:custGeom><a:avLst/><a:gdLst/><a:ahLst/><a:cxnLst/><a:rect l=\"0\" t=\"0\" r=\"r\" b=\"b\"/><a:pathLst><a:path w=\"{}\" h=\"{}\">{commands}</a:path></a:pathLst></a:custGeom>",
                emu(path.width.max(0.01)),
                emu(path.height.max(0.01))
            );
        }
    };
    format!("<a:prstGeom prst=\"{preset}\"><a:avLst/></a:prstGeom>")
}

/// A bullet as Word writes it: the round bullet in Symbol and the square in
/// Wingdings, at their private-use code points; others stay as they are.
fn word_bullet(marker: &str) -> (Option<&'static str>, String) {
    match marker.trim() {
        "\u{2022}" | "\u{25CF}" => (Some("Symbol"), "\u{F0B7}".to_string()),
        "\u{25AA}" | "\u{25A0}" => (Some("Wingdings"), "\u{F0A7}".to_string()),
        _ => (None, marker.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::font_family;

    #[test]
    fn postscript_names_become_families() {
        assert_eq!(font_family("HelveticaNeue-Bold"), "Helvetica Neue");
        assert_eq!(font_family("ComicSansMS"), "Comic Sans MS");
        assert_eq!(font_family("TimesNewRomanPSMT"), "Times New Roman");
        assert_eq!(font_family("ArialMT"), "Arial");
        assert_eq!(font_family("Menlo-Regular"), "Menlo");
        assert_eq!(font_family("CourierNewPSMT"), "Courier New");
        assert_eq!(font_family("Georgia"), "Georgia");
        assert_eq!(font_family("Nonexistent Sans"), "Nonexistent Sans");
        assert_eq!(font_family("AvenirNext-DemiBold"), "Avenir Next");
        assert_eq!(font_family("STHeitiSC-Light"), "ST Heiti SC");
    }
}
