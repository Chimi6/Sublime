//! Renders the document model as a `.docx` package: `document.xml`,
//! `styles.xml`, `numbering.xml`, `footnotes.xml`, `settings.xml`, the
//! header and footer parts, the media files, their relationships, and the
//! content types. Style names are kept, so a reader on the other side
//! (Word, Google Docs) sees the document's own style names.
//!
//! Units: the model's points become twentieths of a point for spacing and
//! indents, half-points for font sizes, and EMUs for drawings.

use std::fmt::Write as _;
use std::io;

use crate::document::{
    Alignment, Anchor, AnchorBase, Baseline, Block, Caps, Document, FloatingContent, Inline,
    InlineImage, LineSpacing, ListLabel, Merge, NumberKind, Paragraph, ParagraphProperties,
    Placement, RevisionKind, Run, RunProperties, Section, SectionStart, Table, mathml_text,
};
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const R: &str = r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const DRAWING: &str = r#"xmlns:wp="http://schemas.openxmlformats.org/drawingml/2006/wordprocessingDrawing" xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" xmlns:pic="http://schemas.openxmlformats.org/drawingml/2006/picture" xmlns:wps="http://schemas.microsoft.com/office/word/2010/wordprocessingShape""#;
const SHAPE: &str = "http://schemas.microsoft.com/office/word/2010/wordprocessingShape";
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
const PICTURE: &str = "http://schemas.openxmlformats.org/drawingml/2006/picture";

/// Writes `document` as a `.docx` to `sink`.
pub fn write_docx<W: io::Write>(document: &Document, sink: W) -> io::Result<W> {
    let mut writer = DocxWriter {
        document,
        body: String::new(),
        relationships: Vec::new(),
        numbering: Numbering::default(),
        page_parts: Vec::new(),
        media_targets: vec![None; document.media.len()],
        drawings: 0,
        even_pages: false,
        floating_done: vec![false; document.floating.len()],
        revisions: 0,
    };
    writer.render_body();
    let footnotes = if document.footnotes.is_empty() {
        None
    } else {
        Some(writer.footnotes_part())
    };
    let mut zip = ZipWriter::new(sink);
    zip.add_deflated("[Content_Types].xml", writer.content_types().as_bytes())?;
    zip.add_deflated("_rels/.rels", root_relationships().as_bytes())?;
    zip.add_deflated("word/document.xml", writer.document_xml().as_bytes())?;
    zip.add_deflated(
        "word/_rels/document.xml.rels",
        writer
            .relationships_xml(&writer.relationships, true)
            .as_bytes(),
    )?;
    zip.add_deflated("word/styles.xml", writer.styles_xml().as_bytes())?;
    if !writer.numbering.instances.is_empty() {
        zip.add_deflated("word/numbering.xml", writer.numbering_xml().as_bytes())?;
    }
    if let Some(footnotes) = &footnotes {
        zip.add_deflated("word/footnotes.xml", footnotes.xml.as_bytes())?;
        if !footnotes.relationships.is_empty() {
            zip.add_deflated(
                "word/_rels/footnotes.xml.rels",
                writer
                    .relationships_xml(&footnotes.relationships, false)
                    .as_bytes(),
            )?;
        }
    }
    if writer.even_pages {
        zip.add_deflated("word/settings.xml", settings_xml().as_bytes())?;
    }
    for part in &writer.page_parts {
        zip.add_deflated(&format!("word/{}", part.name), part.xml.as_bytes())?;
        if !part.relationships.is_empty() {
            zip.add_deflated(
                &format!("word/_rels/{}.rels", part.name),
                writer
                    .relationships_xml(&part.relationships, false)
                    .as_bytes(),
            )?;
        }
    }
    for (index, target) in writer.media_targets.iter().enumerate() {
        if let Some(target) = target {
            zip.add_deflated(&format!("word/{target}"), &document.media[index].bytes)?;
        }
    }
    zip.finish()
}

struct DocxWriter<'d> {
    document: &'d Document,
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
    /// Some section has an even-page header or footer.
    even_pages: bool,
    /// Which floating objects have been anchored to a paragraph.
    floating_done: Vec<bool>,
    /// Tracked changes written so far, for unique ids.
    revisions: u32,
}

/// A relationship of a part: what it links to and how.
#[derive(PartialEq)]
struct Relationship {
    kind: RelationshipKind,
    target: String,
}

#[derive(PartialEq, Clone, Copy)]
enum RelationshipKind {
    Hyperlink,
    Image,
    Header,
    Footer,
}

/// A part rendered on its own, with its own relationships.
struct Part {
    name: String,
    xml: String,
    relationships: Vec<Relationship>,
}

/// One `w:num` per list start: which list style it uses.
#[derive(Default)]
struct Numbering {
    instances: Vec<usize>,
    /// The current instance for each list style, until a list restarts.
    current: Vec<Option<usize>>,
}

impl DocxWriter<'_> {
    fn render_body(&mut self) {
        let mut body = String::new();
        // Floating objects are anchored to the first paragraph on their
        // page, counting the page breaks the document spells out.
        let mut page = 0u32;
        for (index, section) in self.document.sections.iter().enumerate() {
            let last = index + 1 == self.document.sections.len();
            if index > 0 && section.start == SectionStart::NewPage {
                page += 1;
            }
            let properties = self.section_properties(section);
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
                        let anchors = self.floating_due(page, last && is_last_block);
                        let trailer = if is_last_block && !last {
                            properties.as_str()
                        } else {
                            ""
                        };
                        self.render_paragraph(paragraph, &mut body, trailer, &anchors);
                    }
                    Block::Table(table) => self.render_table(table, &mut body),
                }
            }
            // A section's properties ride in its last paragraph, as Word
            // and Pages both write them; a section ending otherwise gets
            // an empty one.
            let ends_with_paragraph = matches!(section.blocks.last(), Some(Block::Paragraph(_)));
            if !ends_with_paragraph {
                let anchors = self.floating_due(page, last);
                self.render_paragraph(&Paragraph::default(), &mut body, &properties, &anchors);
            } else if last {
                body.push_str(&properties);
            }
        }
        self.body = body;
    }

    /// The floating objects to anchor now: those on `page` or before it
    /// that have no anchor yet, and every remaining one at the very end.
    fn floating_due(&mut self, page: u32, all: bool) -> Vec<usize> {
        let mut due = Vec::new();
        for (index, object) in self.document.floating.iter().enumerate() {
            if self.floating_done[index] {
                continue;
            }
            if all || object.page <= page {
                self.floating_done[index] = true;
                due.push(index);
            }
        }
        due
    }

    fn render_blocks(&mut self, blocks: &[Block], out: &mut String) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.render_paragraph(paragraph, out, "", &[]),
                Block::Table(table) => self.render_table(table, out),
            }
        }
    }

    #[inline(never)]
    fn render_table(&mut self, table: &Table, out: &mut String) {
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
            "<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"{total}\" w:type=\"dxa\"/><w:tblLayout w:type=\"fixed\"/></w:tblPr><w:tblGrid>"
        );
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
                if let Some(background) = cell.background {
                    let _ = write!(
                        out,
                        "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                        background.hex()
                    );
                }
                out.push_str("</w:tcPr>");
                // A cell holds at least one paragraph, and ends with one.
                let ends_with_paragraph = matches!(cell.blocks.last(), Some(Block::Paragraph(_)));
                if cell.blocks.is_empty() {
                    out.push_str("<w:p/>");
                } else {
                    self.render_blocks(&cell.blocks, out);
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
                style_id(&self.document.styles.paragraph[style].name)
            );
        }
        if let Some(item) = paragraph.list {
            let instance = self.numbering.instance_for(item.style, item.starts_list);
            let _ = write!(
                properties,
                "<w:numPr><w:ilvl w:val=\"{}\"/><w:numId w:val=\"{}\"/></w:numPr>",
                item.level,
                instance + 1
            );
        }
        paragraph_properties_xml(&paragraph.properties, &mut properties);
        let mut mark = String::new();
        run_properties_xml(&paragraph.run_properties, &mut mark);
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
            self.render_floating(*anchor, out);
        }
        let mut open_link: Option<&str> = None;
        for run in &paragraph.runs {
            if run.link.as_deref() != open_link {
                if open_link.is_some() {
                    out.push_str("</w:hyperlink>");
                }
                open_link = run.link.as_deref();
                if let Some(target) = open_link {
                    let id = self.relationship_for(RelationshipKind::Hyperlink, target);
                    let _ = write!(out, "<w:hyperlink r:id=\"rId{id}\">");
                }
            }
            // A tracked change wraps its run.
            let change = run.revision.as_ref().map(|revision| {
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
            self.render_run(paragraph, run, out);
            if let Some(element) = change {
                let _ = write!(out, "</{element}>");
            }
        }
        if open_link.is_some() {
            out.push_str("</w:hyperlink>");
        }
        out.push_str("</w:p>");
    }

    fn render_run(&mut self, paragraph: &Paragraph, run: &Run, out: &mut String) {
        let mut properties = String::new();
        if let Some(style) = run.style {
            let _ = write!(
                properties,
                "<w:rStyle w:val=\"{}\"/>",
                style_id(&self.document.styles.character[style].name)
            );
        }
        let mut merged = paragraph.run_properties.clone();
        merged.overlay(&run.properties);
        run_properties_xml(&merged, &mut properties);
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
        out.push_str("<w:r>");
        out.push_str(&properties);
        let deleted = run
            .revision
            .as_ref()
            .is_some_and(|revision| revision.kind == RevisionKind::Deletion);
        match &run.content {
            Inline::Text(text) => {
                let element = if deleted { "w:delText" } else { "w:t" };
                let _ = write!(out, "<{element} xml:space=\"preserve\">");
                escape_text(out, text);
                let _ = write!(out, "</{element}>");
            }
            Inline::PageBreak => out.push_str("<w:br w:type=\"page\"/>"),
            // Equations are written as their text until Office Math is
            // supported.
            Inline::Math(mathml) => {
                out.push_str("<w:t xml:space=\"preserve\">");
                escape_text(out, &mathml_text(mathml));
                out.push_str("</w:t>");
            }
            Inline::LineBreak => out.push_str("<w:br/>"),
            Inline::Tab => out.push_str("<w:tab/>"),
            Inline::Footnote(note) => {
                let _ = write!(out, "<w:footnoteReference w:id=\"{}\"/>", note + 1);
            }
            Inline::Image(image) => self.render_image(image, out),
            Inline::PageNumber | Inline::PageCount => {}
        }
        out.push_str("</w:r>");
    }

    /// A picture, inline or anchored beside the text. Media Word cannot
    /// show as a picture (such as PDF) is left out.
    #[inline(never)]
    fn render_image(&mut self, image: &InlineImage, out: &mut String) {
        let Some(target) = self.media_target(image.media) else {
            return;
        };
        let file_name = target.rsplit('/').next().unwrap_or("image").to_string();
        let id = self.relationship_for(RelationshipKind::Image, &target);
        self.drawings += 1;
        let number = self.drawings;
        let width = emu(image.width);
        let height = emu(image.height);
        let mut description = String::new();
        escape_attribute(&mut description, image.description.as_deref().unwrap_or(""));
        let mut name = String::new();
        escape_attribute(&mut name, &file_name);
        let extent = format!(
            "<wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/>"
        );
        let graphic = format!(
            "<wp:docPr id=\"{number}\" name=\"{name}\" descr=\"{description}\"/><wp:cNvGraphicFramePr><a:graphicFrameLocks noChangeAspect=\"1\"/></wp:cNvGraphicFramePr><a:graphic><a:graphicData uri=\"{PICTURE}\"><pic:pic><pic:nvPicPr><pic:cNvPr id=\"{number}\" name=\"{name}\"/><pic:cNvPicPr/></pic:nvPicPr><pic:blipFill><a:blip r:embed=\"rId{id}\"/><a:stretch><a:fillRect/></a:stretch></pic:blipFill><pic:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom></pic:spPr></pic:pic></a:graphicData></a:graphic>"
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
    fn render_floating(&mut self, index: usize, out: &mut String) {
        let object = &self.document.floating[index];
        let horizontal = Anchor {
            from: AnchorBase::Page,
            offset: object.x,
        };
        let vertical = Anchor {
            from: AnchorBase::Page,
            offset: object.y,
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
                };
                out.push_str("<w:r>");
                self.render_image(&image, out);
                out.push_str("</w:r>");
            }
            FloatingContent::TextBox { blocks, fill } => {
                let mut content = String::new();
                self.render_blocks(blocks, &mut content);
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
                let _ = write!(
                    out,
                    "<w:r><w:drawing><wp:anchor distT=\"0\" distB=\"0\" distL=\"0\" distR=\"0\" simplePos=\"0\" relativeHeight=\"{}\" behindDoc=\"0\" locked=\"0\" layoutInCell=\"1\" allowOverlap=\"1\"><wp:simplePos x=\"0\" y=\"0\"/><wp:positionH relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionH><wp:positionV relativeFrom=\"page\"><wp:posOffset>{}</wp:posOffset></wp:positionV><wp:extent cx=\"{width}\" cy=\"{height}\"/><wp:effectExtent l=\"0\" t=\"0\" r=\"0\" b=\"0\"/><wp:wrapSquare wrapText=\"bothSides\"/><wp:docPr id=\"{number}\" name=\"Text Box {number}\"/><wp:cNvGraphicFramePr/><a:graphic><a:graphicData uri=\"{SHAPE}\"><wps:wsp><wps:cNvSpPr txBox=\"1\"/><wps:spPr><a:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"{width}\" cy=\"{height}\"/></a:xfrm><a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>{fill_xml}<a:ln><a:noFill/></a:ln></wps:spPr><wps:txbx><w:txbxContent>{content}</w:txbxContent></wps:txbx><wps:bodyPr wrap=\"square\" lIns=\"50800\" tIns=\"50800\" rIns=\"50800\" bIns=\"50800\" anchor=\"t\"><a:noAutofit/></wps:bodyPr></wps:wsp></a:graphicData></a:graphic></wp:anchor></w:drawing></w:r>",
                    251_658_240 + number,
                    emu(object.x),
                    emu(object.y),
                );
            }
        }
    }

    /// The package path of a media file under `word/`, when Word can show
    /// it as a picture.
    #[inline(never)]
    fn media_target(&mut self, media: usize) -> Option<String> {
        if let Some(Some(target)) = self.media_targets.get(media) {
            return Some(target.clone());
        }
        let file = self.document.media.get(media)?;
        let extension = file
            .name
            .rsplit('.')
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        image_content_type(&extension)?;
        let target = format!("media/image{}.{extension}", media + 1);
        self.media_targets[media] = Some(target.clone());
        Some(target)
    }

    fn relationship_for(&mut self, kind: RelationshipKind, target: &str) -> usize {
        let wanted = Relationship {
            kind,
            target: target.to_string(),
        };
        if let Some(index) = self
            .relationships
            .iter()
            .position(|existing| *existing == wanted)
        {
            return index + 10;
        }
        self.relationships.push(wanted);
        self.relationships.len() - 1 + 10
    }

    /// Renders blocks as a part of their own, with their own relationships.
    fn render_part(&mut self, blocks: &[Block]) -> (String, Vec<Relationship>) {
        let outer = std::mem::take(&mut self.relationships);
        let mut xml = String::new();
        self.render_blocks(blocks, &mut xml);
        let relationships = std::mem::replace(&mut self.relationships, outer);
        (xml, relationships)
    }

    /// A header or footer part; the relationship id the section refers
    /// to it by.
    #[inline(never)]
    fn page_part(&mut self, blocks: &[Block], kind: RelationshipKind) -> usize {
        let (inner, relationships) = self.render_part(blocks);
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
    fn section_properties(&mut self, section: &Section) -> String {
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
                ("default", &variant.default),
                ("first", &variant.first),
                ("even", &variant.even),
            ];
            for (page_kind, blocks) in pages {
                let Some(blocks) = blocks else {
                    continue;
                };
                let id = self.page_part(blocks, kind);
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
        if section.columns > 1 {
            let _ = write!(
                xml,
                "<w:cols w:num=\"{}\" w:space=\"708\"/>",
                section.columns
            );
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

    fn document_xml(&self) -> String {
        let mut xml = String::with_capacity(self.body.len() + 512);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:document {W} {R} {DRAWING}><w:body>");
        xml.push_str(&self.body);
        xml.push_str("</w:body></w:document>");
        xml
    }

    /// The relationships of a part; the document part also links the
    /// styles, numbering, footnotes, and settings parts.
    fn relationships_xml(&self, relationships: &[Relationship], document: bool) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<Relationships xmlns=\"{PACKAGE_REL}\">");
        if document {
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
            if !self.document.footnotes.is_empty() {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId3\" Type=\"{REL}/footnotes\" Target=\"footnotes.xml\"/>"
                );
            }
            if self.even_pages {
                let _ = write!(
                    xml,
                    "<Relationship Id=\"rId4\" Type=\"{REL}/settings\" Target=\"settings.xml\"/>"
                );
            }
        }
        for (index, relationship) in relationships.iter().enumerate() {
            let kind = match relationship.kind {
                RelationshipKind::Hyperlink => "hyperlink",
                RelationshipKind::Image => "image",
                RelationshipKind::Header => "header",
                RelationshipKind::Footer => "footer",
            };
            let _ = write!(
                xml,
                "<Relationship Id=\"rId{}\" Type=\"{REL}/{kind}\" Target=\"",
                index + 10
            );
            escape_attribute(&mut xml, &relationship.target);
            xml.push('"');
            if relationship.kind == RelationshipKind::Hyperlink {
                xml.push_str(" TargetMode=\"External\"");
            }
            xml.push_str("/>");
        }
        xml.push_str("</Relationships>");
        xml
    }

    fn content_types(&self) -> String {
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
        if !self.document.footnotes.is_empty() {
            xml.push_str("<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>");
        }
        if self.even_pages {
            xml.push_str("<Override PartName=\"/word/settings.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.settings+xml\"/>");
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

    fn styles_xml(&self) -> String {
        let styles = &self.document.styles;
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:styles {W}>");
        xml.push_str("<w:docDefaults><w:rPrDefault><w:rPr><w:sz w:val=\"22\"/></w:rPr></w:rPrDefault><w:pPrDefault><w:pPr/></w:pPrDefault></w:docDefaults>");
        xml.push_str("<w:style w:type=\"paragraph\" w:default=\"1\" w:styleId=\"Normal\"><w:name w:val=\"Normal\"/></w:style>");
        xml.push_str("<w:style w:type=\"character\" w:default=\"1\" w:styleId=\"DefaultParagraphFont\"><w:name w:val=\"Default Paragraph Font\"/></w:style>");
        xml.push_str("<w:style w:type=\"character\" w:styleId=\"Hyperlink\"><w:name w:val=\"Hyperlink\"/><w:rPr><w:color w:val=\"0563C1\"/><w:u w:val=\"single\"/></w:rPr></w:style>");
        xml.push_str("<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:left w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:right w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/></w:tblBorders><w:tblCellMar><w:top w:w=\"80\" w:type=\"dxa\"/><w:left w:w=\"80\" w:type=\"dxa\"/><w:bottom w:w=\"80\" w:type=\"dxa\"/><w:right w:w=\"80\" w:type=\"dxa\"/></w:tblCellMar></w:tblPr></w:style>");
        for style in &styles.paragraph {
            let _ = write!(
                xml,
                "<w:style w:type=\"paragraph\" w:styleId=\"{}\"><w:name w:val=\"",
                style_id(&style.name)
            );
            escape_attribute(&mut xml, &style.name);
            xml.push_str("\"/>");
            if let Some(parent) = style.parent {
                let _ = write!(
                    xml,
                    "<w:basedOn w:val=\"{}\"/>",
                    style_id(&styles.paragraph[parent].name)
                );
            }
            let mut paragraph = String::new();
            paragraph_properties_xml(&style.paragraph, &mut paragraph);
            if !paragraph.is_empty() {
                let _ = write!(xml, "<w:pPr>{paragraph}</w:pPr>");
            }
            let mut run = String::new();
            run_properties_xml(&style.run, &mut run);
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
            run_properties_xml(&style.run, &mut run);
            if !run.is_empty() {
                let _ = write!(xml, "<w:rPr>{run}</w:rPr>");
            }
            xml.push_str("</w:style>");
        }
        xml.push_str("</w:styles>");
        xml
    }

    fn numbering_xml(&self) -> String {
        let styles = &self.document.styles.list;
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:numbering {W}>");
        for (index, style) in styles.iter().enumerate() {
            let _ = write!(
                xml,
                "<w:abstractNum w:abstractNumId=\"{index}\"><w:multiLevelType w:val=\"hybridMultilevel\"/>"
            );
            for (level, definition) in style.levels.iter().enumerate().take(9) {
                let (format, text) = match &definition.label {
                    ListLabel::None => ("none", String::new()),
                    ListLabel::Text(marker) => ("bullet", marker.clone()),
                    ListLabel::Number(number) => (
                        match number.kind {
                            NumberKind::Decimal => "decimal",
                            NumberKind::LowerLetter => "lowerLetter",
                            NumberKind::UpperLetter => "upperLetter",
                            NumberKind::LowerRoman => "lowerRoman",
                            NumberKind::UpperRoman => "upperRoman",
                        },
                        number.pattern.replace("%1", &format!("%{}", level + 1)),
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
                    "\"/><w:lvlJc w:val=\"left\"/><w:pPr><w:ind w:left=\"{left}\" w:hanging=\"{hanging}\"/></w:pPr></w:lvl>"
                );
            }
            xml.push_str("</w:abstractNum>");
        }
        for (index, style) in self.numbering.instances.iter().enumerate() {
            let _ = write!(
                xml,
                "<w:num w:numId=\"{}\"><w:abstractNumId w:val=\"{style}\"/><w:lvlOverride w:ilvl=\"0\"><w:startOverride w:val=\"1\"/></w:lvlOverride></w:num>",
                index + 1
            );
        }
        xml.push_str("</w:numbering>");
        xml
    }

    fn footnotes_part(&mut self) -> Part {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:footnotes {W} {R} {DRAWING}>");
        xml.push_str("<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>");
        xml.push_str("<w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
        let outer = std::mem::take(&mut self.relationships);
        for (index, note) in self.document.footnotes.iter().enumerate() {
            let _ = write!(xml, "<w:footnote w:id=\"{}\">", index + 1);
            let mut body = String::new();
            self.render_blocks(&note.blocks, &mut body);
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
    fn instance_for(&mut self, style: usize, starts_list: bool) -> usize {
        if self.current.len() <= style {
            self.current.resize(style + 1, None);
        }
        match self.current[style] {
            Some(instance) if !starts_list => instance,
            _ => {
                self.instances.push(style);
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

fn settings_xml() -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?><w:settings {W}><w:evenAndOddHeaders/></w:settings>"
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

fn paragraph_properties_xml(properties: &ParagraphProperties, out: &mut String) {
    if properties.keep_with_next == Some(true) {
        out.push_str("<w:keepNext/>");
    }
    if properties.keep_lines_together == Some(true) {
        out.push_str("<w:keepLines/>");
    }
    if let Some(widows) = properties.widow_control {
        let _ = write!(
            out,
            "<w:widowControl w:val=\"{}\"/>",
            if widows { "1" } else { "0" }
        );
    }
    if let Some(background) = properties.background {
        let _ = write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            background.hex()
        );
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

fn run_properties_xml(properties: &RunProperties, out: &mut String) {
    if let Some(font) = &properties.font {
        let family = font_family(font);
        out.push_str("<w:rFonts w:ascii=\"");
        escape_attribute(out, family);
        out.push_str("\" w:hAnsi=\"");
        escape_attribute(out, family);
        out.push_str("\" w:cs=\"");
        escape_attribute(out, family);
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
    if let Some(color) = properties.color {
        let _ = write!(out, "<w:color w:val=\"{}\"/>", color.hex());
    }
    if let Some(size) = properties.size {
        let half_points = (size * 2.0).round() as i64;
        let _ = write!(
            out,
            "<w:sz w:val=\"{half_points}\"/><w:szCs w:val=\"{half_points}\"/>"
        );
    }
    if let Some(highlight) = properties.highlight {
        let _ = write!(
            out,
            "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
            highlight.hex()
        );
    }
    if let Some(underline) = properties.underline {
        out.push_str(if underline {
            "<w:u w:val=\"single\"/>"
        } else {
            "<w:u w:val=\"none\"/>"
        });
    }
    match properties.baseline {
        Some(Baseline::Superscript) => out.push_str("<w:vertAlign w:val=\"superscript\"/>"),
        Some(Baseline::Subscript) => out.push_str("<w:vertAlign w:val=\"subscript\"/>"),
        None => {}
    }
    if let Some(language) = &properties.language {
        out.push_str("<w:lang w:val=\"");
        escape_attribute(out, language);
        out.push_str("\"/>");
    }
}

/// Pages names fonts by PostScript name (`HelveticaNeue-Bold`); Word wants
/// the family. The style suffix after the hyphen is dropped, and a few
/// common PostScript families are spelled the way Word knows them.
fn font_family(postscript_name: &str) -> &str {
    let family = postscript_name.split('-').next().unwrap_or(postscript_name);
    match family {
        "HelveticaNeue" => "Helvetica Neue",
        "TimesNewRomanPSMT" | "TimesNewRoman" => "Times New Roman",
        "CourierNewPSMT" | "CourierNew" => "Courier New",
        "ArialMT" => "Arial",
        other => other,
    }
}

/// The style identifier: the name itself, as Pages exports it (Word
/// accepts spaces), with only the characters an attribute cannot hold
/// removed.
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
