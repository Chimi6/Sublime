//! Renders the document model as a `.docx` package: `document.xml`,
//! `styles.xml`, `numbering.xml`, `footnotes.xml`, their relationships,
//! and the content types. Style names are kept, so a reader on the other
//! side (Word, Google Docs) sees the document's own style names.
//!
//! Units: the model's points become twentieths of a point for spacing and
//! indents, and half-points for font sizes.

use std::fmt::Write as _;
use std::io;

use crate::document::{
    Alignment, Baseline, Block, Caps, Document, Inline, LineSpacing, ListLabel, NumberKind,
    Paragraph, ParagraphProperties, Run, RunProperties, Section,
};
use crate::io::xml::{escape_attribute, escape_text};
use crate::io::zip::ZipWriter;

const W: &str = r#"xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main""#;
const R: &str = r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships""#;
const REL: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
const PACKAGE_REL: &str = "http://schemas.openxmlformats.org/package/2006/relationships";

/// Writes `document` as a `.docx` to `sink`.
pub fn write_docx<W: io::Write>(document: &Document, sink: W) -> io::Result<W> {
    let mut writer = DocxWriter {
        document,
        body: String::new(),
        relationships: Vec::new(),
        numbering: Numbering::default(),
    };
    writer.render_body();
    let mut zip = ZipWriter::new(sink);
    zip.add_deflated("[Content_Types].xml", writer.content_types().as_bytes())?;
    zip.add_deflated("_rels/.rels", root_relationships().as_bytes())?;
    zip.add_deflated("word/document.xml", writer.document_xml().as_bytes())?;
    zip.add_deflated(
        "word/_rels/document.xml.rels",
        writer.document_relationships().as_bytes(),
    )?;
    zip.add_deflated("word/styles.xml", writer.styles_xml().as_bytes())?;
    if !writer.numbering.instances.is_empty() {
        zip.add_deflated("word/numbering.xml", writer.numbering_xml().as_bytes())?;
    }
    if !document.footnotes.is_empty() {
        zip.add_deflated("word/footnotes.xml", writer.footnotes_xml().as_bytes())?;
    }
    zip.finish()
}

struct DocxWriter<'d> {
    document: &'d Document,
    body: String,
    /// External hyperlink targets, one relationship each: `rId<n+10>`.
    relationships: Vec<String>,
    numbering: Numbering,
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
        for (index, section) in self.document.sections.iter().enumerate() {
            let last = index + 1 == self.document.sections.len();
            self.render_blocks(&section.blocks, &mut body);
            if last {
                body.push_str(&section_properties(section));
            } else {
                body.push_str("<w:p><w:pPr>");
                body.push_str(&section_properties(section));
                body.push_str("</w:pPr></w:p>");
            }
        }
        self.body = body;
    }

    fn render_blocks(&mut self, blocks: &[Block], out: &mut String) {
        for block in blocks {
            match block {
                Block::Paragraph(paragraph) => self.render_paragraph(paragraph, out),
                Block::Table(table) => {
                    out.push_str("<w:tbl><w:tblPr><w:tblStyle w:val=\"TableGrid\"/><w:tblW w:w=\"0\" w:type=\"auto\"/></w:tblPr>");
                    for row in &table.rows {
                        out.push_str("<w:tr>");
                        for cell in &row.cells {
                            out.push_str("<w:tc><w:tcPr>");
                            if cell.column_span > 1 {
                                let _ = write!(out, "<w:gridSpan w:val=\"{}\"/>", cell.column_span);
                            }
                            if let Some(background) = cell.background {
                                let _ = write!(
                                    out,
                                    "<w:shd w:val=\"clear\" w:color=\"auto\" w:fill=\"{}\"/>",
                                    background.hex()
                                );
                            }
                            out.push_str("</w:tcPr>");
                            if cell.blocks.is_empty() {
                                out.push_str("<w:p/>");
                            } else {
                                self.render_blocks(&cell.blocks, out);
                            }
                            out.push_str("</w:tc>");
                        }
                        out.push_str("</w:tr>");
                    }
                    out.push_str("</w:tbl>");
                }
            }
        }
    }

    fn render_paragraph(&mut self, paragraph: &Paragraph, out: &mut String) {
        // A page break is its own paragraph, as Word and Pages both write it.
        if paragraph.page_break_before {
            out.push_str("<w:p>");
            if let Some(style) = paragraph.style {
                let _ = write!(
                    out,
                    "<w:pPr><w:pStyle w:val=\"{}\"/></w:pPr>",
                    style_id(&self.document.styles.paragraph[style].name)
                );
            }
            out.push_str("<w:r><w:br w:type=\"page\"/></w:r></w:p>");
        }
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
        if !properties.is_empty() {
            out.push_str("<w:pPr>");
            out.push_str(&properties);
            out.push_str("</w:pPr>");
        }
        let mut open_link: Option<&str> = None;
        for run in &paragraph.runs {
            if run.link.as_deref() != open_link {
                if open_link.is_some() {
                    out.push_str("</w:hyperlink>");
                }
                open_link = run.link.as_deref();
                if let Some(target) = open_link {
                    let id = self.relationship_for(target);
                    let _ = write!(out, "<w:hyperlink r:id=\"rId{id}\">");
                }
            }
            self.render_run(paragraph, run, out);
        }
        if open_link.is_some() {
            out.push_str("</w:hyperlink>");
        }
        out.push_str("</w:p>");
    }

    fn render_run(&mut self, paragraph: &Paragraph, run: &Run, out: &mut String) {
        out.push_str("<w:r>");
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
        if !properties.is_empty() {
            out.push_str("<w:rPr>");
            out.push_str(&properties);
            out.push_str("</w:rPr>");
        }
        match &run.content {
            Inline::Text(text) => {
                out.push_str("<w:t xml:space=\"preserve\">");
                escape_text(out, text);
                out.push_str("</w:t>");
            }
            Inline::LineBreak => out.push_str("<w:br/>"),
            Inline::Tab => out.push_str("<w:tab/>"),
            Inline::Footnote(note) => {
                let _ = write!(out, "<w:footnoteReference w:id=\"{}\"/>", note + 1);
            }
            Inline::Image(_) => {}
        }
        out.push_str("</w:r>");
    }

    fn relationship_for(&mut self, target: &str) -> usize {
        if let Some(index) = self
            .relationships
            .iter()
            .position(|existing| existing == target)
        {
            return index + 10;
        }
        self.relationships.push(target.to_string());
        self.relationships.len() - 1 + 10
    }

    fn document_xml(&self) -> String {
        let mut xml = String::with_capacity(self.body.len() + 512);
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:document {W} {R}><w:body>");
        xml.push_str(&self.body);
        xml.push_str("</w:body></w:document>");
        xml
    }

    fn document_relationships(&self) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<Relationships xmlns=\"{PACKAGE_REL}\">");
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
        for (index, target) in self.relationships.iter().enumerate() {
            let _ = write!(
                xml,
                "<Relationship Id=\"rId{}\" Type=\"{REL}/hyperlink\" Target=\"",
                index + 10
            );
            escape_attribute(&mut xml, target);
            xml.push_str("\" TargetMode=\"External\"/>");
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
        xml.push_str("<Override PartName=\"/word/document.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml\"/>");
        xml.push_str("<Override PartName=\"/word/styles.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.styles+xml\"/>");
        if !self.numbering.instances.is_empty() {
            xml.push_str("<Override PartName=\"/word/numbering.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.numbering+xml\"/>");
        }
        if !self.document.footnotes.is_empty() {
            xml.push_str("<Override PartName=\"/word/footnotes.xml\" ContentType=\"application/vnd.openxmlformats-officedocument.wordprocessingml.footnotes+xml\"/>");
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
        xml.push_str("<w:style w:type=\"table\" w:styleId=\"TableGrid\"><w:name w:val=\"Table Grid\"/><w:tblPr><w:tblBorders><w:top w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:left w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:bottom w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:right w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideH w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/><w:insideV w:val=\"single\" w:sz=\"4\" w:color=\"000000\"/></w:tblBorders></w:tblPr></w:style>");
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

    fn footnotes_xml(&mut self) -> String {
        let mut xml = String::new();
        xml.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>");
        let _ = write!(xml, "<w:footnotes {W} {R}>");
        xml.push_str("<w:footnote w:type=\"separator\" w:id=\"-1\"><w:p><w:r><w:separator/></w:r></w:p></w:footnote>");
        xml.push_str("<w:footnote w:type=\"continuationSeparator\" w:id=\"0\"><w:p><w:r><w:continuationSeparator/></w:r></w:p></w:footnote>");
        let notes = self.document.footnotes.clone();
        for (index, note) in notes.iter().enumerate() {
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
        xml
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

fn section_properties(section: &Section) -> String {
    let page = &section.page;
    let mut xml = String::new();
    let _ = write!(
        xml,
        "<w:sectPr><w:pgSz w:w=\"{}\" w:h=\"{}\"/>",
        twips(page.width),
        twips(page.height)
    );
    let _ = write!(
        xml,
        "<w:pgMar w:top=\"{}\" w:right=\"{}\" w:bottom=\"{}\" w:left=\"{}\" w:header=\"708\" w:footer=\"708\" w:gutter=\"0\"/>",
        twips(page.margin_top),
        twips(page.margin_right),
        twips(page.margin_bottom),
        twips(page.margin_left)
    );
    if section.columns > 1 {
        let _ = write!(
            xml,
            "<w:cols w:num=\"{}\" w:space=\"708\"/>",
            section.columns
        );
    }
    xml.push_str("</w:sectPr>");
    xml
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
