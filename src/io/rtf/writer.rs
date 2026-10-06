//! The document model as RTF, in the form Word writes: a font, colour, and
//! style table, list tables with an override per list instance (so restarts
//! and start numbers carry), then the sections. Formatting is written whole
//! on every run and paragraph (RTF's formatting is absolute, so a reader
//! never has to resolve a style to see it), text outside ASCII as `\uN`
//! escapes, and tables as `\trowd` rows with their merges, borders, and
//! shading, nested tables as Word nests them.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::io;

use crate::document::{
    Alignment, Baseline, Block, Border, Caps, Cell, Color, Document, FloatingContent, Id, Inline,
    LineSpacing, ListLabel, Merge, NumberKind, PageVariants, Paragraph, ParagraphProperties,
    RevisionKind, Run, RunProperties, Section, SectionStart, ShapeGeometry, TabAlignment, Table,
    TextWrap, VerticalAlignment,
};

/// Writes `document` as RTF into `sink`.
pub fn write_rtf(document: &Document, sink: &mut dyn io::Write) -> io::Result<()> {
    let mut writer = Writer::new(document, sink);
    writer.write()
}

/// Points to twentieths of a point.
fn twips(points: f32) -> i64 {
    (points * 20.0).round() as i64
}

/// A colour as RTF shapes give it: `0x00BBGGRR`.
fn bgr(color: Color) -> u32 {
    u32::from(color.blue) << 16 | u32::from(color.green) << 8 | u32::from(color.red)
}

/// Where a paragraph sits: the body, or a table cell `depth` tables deep.
#[derive(Clone, Copy)]
struct Place {
    depth: usize,
}

struct Writer<'a> {
    document: &'a Document,
    sink: &'a mut dyn io::Write,
    out: String,
    fonts: Vec<String>,
    font_ids: HashMap<String, usize>,
    colors: Vec<Color>,
    color_ids: HashMap<Color, usize>,
    /// Each paragraph style's RTF number.
    paragraph_numbers: Vec<usize>,
    character_base: usize,
    /// Per paragraph (by address), the list override it uses, from a walk of
    /// the document before writing.
    list_overrides: Vec<ListOverride>,
    instance_of: HashMap<usize, usize>,
    counters: HashMap<usize, Vec<u32>>,
    authors: Vec<String>,
    /// Comments already written (a comment's text follows its range).
    comments_written: Vec<bool>,
    /// Writing a section that a `\sect` ends.
    section_end: bool,
}

/// One `\listoverride`: the list it uses and the number it starts at.
struct ListOverride {
    style: usize,
    start: Option<u32>,
}

impl<'a> Writer<'a> {
    fn new(document: &'a Document, sink: &'a mut dyn io::Write) -> Writer<'a> {
        Writer {
            document,
            sink,
            out: String::with_capacity(64 * 1024),
            fonts: Vec::new(),
            font_ids: HashMap::new(),
            colors: Vec::new(),
            color_ids: HashMap::new(),
            paragraph_numbers: Vec::new(),
            character_base: 0,
            list_overrides: Vec::new(),
            instance_of: HashMap::new(),
            counters: HashMap::new(),
            authors: Vec::new(),
            comments_written: vec![false; document.comments.len()],
            section_end: false,
        }
    }

    fn flush_if_large(&mut self) -> io::Result<()> {
        if self.out.len() > 60 * 1024 {
            self.sink.write_all(self.out.as_bytes())?;
            self.out.clear();
        }
        Ok(())
    }

    fn write(&mut self) -> io::Result<()> {
        self.collect_tables();
        self.plan_lists();
        self.header();
        let document = self.document;
        let sections = &document.sections;
        for (index, section) in sections.iter().enumerate() {
            if index > 0 {
                // `\sect` ends the paragraph before it, as Word writes it: a
                // `\par` there would add an empty paragraph to the section.
                if matches!(
                    sections[index - 1].blocks.last(),
                    Some(Block::Table(_)) | None
                ) {
                    self.out.push_str("\\pard\\plain ");
                }
                self.out.push_str("\\sect");
            }
            self.section(section, index == 0);
            if index == 0 {
                self.unplaced_floating();
            }
            let last = index + 1 == sections.len();
            self.section_end = !last;
            self.blocks(&section.blocks, Place { depth: 0 }, last)?;
            self.section_end = false;
        }
        self.out.push_str("}\n");
        self.sink.write_all(self.out.as_bytes())?;
        self.out.clear();
        self.sink.flush()
    }

    // ----- tables at the head -----

    fn font(&mut self, name: &str) -> usize {
        if let Some(id) = self.font_ids.get(name) {
            return *id;
        }
        self.fonts.push(name.to_string());
        let id = self.fonts.len() - 1;
        self.font_ids.insert(name.to_string(), id);
        id
    }

    fn color(&mut self, color: Color) -> usize {
        if let Some(id) = self.color_ids.get(&color) {
            return *id;
        }
        self.colors.push(color);
        let id = self.colors.len();
        self.color_ids.insert(color, id);
        id
    }

    /// Registers every font and colour the document uses, so the tables at
    /// the head are complete before the body is written.
    fn collect_tables(&mut self) {
        let document = self.document;
        // The default font first: the default style's, or the first used.
        let default_font = document
            .styles
            .default_paragraph
            .and_then(|style| document.paragraph_style_run(style).font)
            .or_else(|| document.run_properties.iter().find_map(|run| run.font))
            .map(|id| document.string(id).to_string())
            .filter(|name| !name.is_empty())
            .unwrap_or_else(|| "Times New Roman".to_string());
        self.font(&default_font);
        let runs: Vec<RunProperties> = document
            .run_properties
            .iter()
            .copied()
            .chain(document.styles.paragraph.iter().map(|style| style.run))
            .chain(document.styles.character.iter().map(|style| style.run))
            .collect();
        for run in runs {
            if let Some(font) = run.font {
                let name = document.string(font).to_string();
                if !name.is_empty() {
                    self.font(&name);
                }
            }
            for color in [run.color, run.highlight].into_iter().flatten() {
                self.color(color);
            }
        }
        let paragraphs: Vec<ParagraphProperties> = document
            .paragraph_properties
            .iter()
            .copied()
            .chain(
                document
                    .styles
                    .paragraph
                    .iter()
                    .map(|style| style.paragraph),
            )
            .collect();
        for paragraph in paragraphs {
            if let Some(color) = paragraph.background {
                self.color(color);
            }
            if let Some(color) = paragraph.border.and_then(|border| border.line.color) {
                self.color(color);
            }
        }
        for section in &document.sections {
            self.collect_block_colors(&section.blocks);
        }
        if let Some(color) = document.page_color {
            self.color(color);
        }
        self.font("Symbol");
        self.font("Wingdings");
        for revision in &document.revisions {
            let author = revision.author.clone().unwrap_or_default();
            if !self.authors.contains(&author) {
                self.authors.push(author);
            }
        }
    }

    fn collect_block_colors(&mut self, blocks: &[Block]) {
        for block in blocks {
            let Block::Table(table) = block else {
                continue;
            };
            if let Some(borders) = table.borders {
                for line in [
                    borders.top,
                    borders.bottom,
                    borders.left,
                    borders.right,
                    borders.inside_horizontal,
                    borders.inside_vertical,
                ]
                .into_iter()
                .flatten()
                {
                    if let Some(color) = line.color {
                        self.color(color);
                    }
                }
            }
            for row in &table.rows {
                for cell in &row.cells {
                    if let Some(color) = cell.background {
                        self.color(color);
                    }
                    for line in [
                        cell.borders.top,
                        cell.borders.bottom,
                        cell.borders.left,
                        cell.borders.right,
                    ]
                    .into_iter()
                    .flatten()
                    .flatten()
                    {
                        if let Some(color) = line.color {
                            self.color(color);
                        }
                    }
                    self.collect_block_colors(&cell.blocks);
                }
            }
        }
    }

    /// Assigns each list instance (each item that starts a list) its own
    /// override, so numbering restarts where the document restarts it.
    fn plan_lists(&mut self) {
        let document = self.document;
        // Every list style gets an override of its own first, so items
        // that continue a list before any item starts one have a home.
        for style in 0..document.styles.list.len() {
            self.list_overrides
                .push(ListOverride { style, start: None });
        }
        let mut paragraphs: Vec<&Paragraph> = Vec::new();
        for section in &document.sections {
            collect_paragraphs(&section.blocks, &mut paragraphs);
            for part in [&section.headers, &section.footers] {
                for blocks in [&part.default, &part.first, &part.even]
                    .into_iter()
                    .flatten()
                {
                    collect_paragraphs(blocks, &mut paragraphs);
                }
            }
        }
        let mut current: HashMap<usize, usize> = HashMap::new();
        for paragraph in paragraphs {
            let Some(item) = paragraph.list else {
                continue;
            };
            let instance = if item.starts_list || !current.contains_key(&item.style) {
                let start = (item.start != 1).then_some(item.start);
                let reuse = !current.contains_key(&item.style) && start.is_none();
                let index = if reuse {
                    item.style
                } else {
                    self.list_overrides.push(ListOverride {
                        style: item.style,
                        start,
                    });
                    self.list_overrides.len() - 1
                };
                current.insert(item.style, index);
                index
            } else {
                current[&item.style]
            };
            self.instance_of
                .insert(paragraph as *const Paragraph as usize, instance);
        }
    }

    fn header(&mut self) {
        // `\htmautsp` as Word writes it: LibreOffice otherwise spaces
        // paragraphs by older rules.
        let mut head = String::from("{\\rtf1\\ansi\\ansicpg1252\\deff0\\uc1\\htmautsp\\deftab720");
        head.push_str("{\\fonttbl");
        for (index, name) in self.fonts.iter().enumerate() {
            let charset = if name == "Symbol" || name.starts_with("Wingdings") {
                2
            } else {
                0
            };
            let _ = write!(head, "{{\\f{index}\\fnil\\fcharset{charset} ");
            escape_into(&mut head, name);
            head.push_str(";}");
        }
        head.push_str("}\n{\\colortbl;");
        for color in &self.colors {
            let _ = write!(
                head,
                "\\red{}\\green{}\\blue{};",
                color.red, color.green, color.blue
            );
        }
        head.push_str("}\n");
        self.out.push_str(&head);
        self.stylesheet();
        self.list_tables();
        if !self.authors.is_empty() {
            self.out.push_str("{\\*\\revtbl {Unknown;}");
            let authors = self.authors.clone();
            for author in authors {
                self.out.push('{');
                let name = if author.is_empty() {
                    "Unknown".to_string()
                } else {
                    author
                };
                escape_into(&mut self.out, &name);
                self.out.push_str(";}");
            }
            self.out.push_str("}\n");
        }
        let document = self.document;
        if let Some(section) = document.sections.first() {
            let page = &section.page;
            let _ = write!(
                self.out,
                "\\paperw{}\\paperh{}\\margl{}\\margr{}\\margt{}\\margb{}",
                twips(page.width),
                twips(page.height),
                twips(page.margin_left),
                twips(page.margin_right),
                twips(page.margin_top),
                twips(page.margin_bottom)
            );
        }
        let facing = document
            .sections
            .iter()
            .any(|section| section.headers.even.is_some() || section.footers.even.is_some());
        if facing {
            self.out.push_str("\\facingp");
        }
        if let Some(color) = document.page_color {
            let _ = write!(
                self.out,
                "\\viewbksp1{{\\*\\background{{\\shp{{\\*\\shpinst\\shpleft0\\shptop0\\shpright0\\shpbottom0\\shpfhdr0\\shpbxmargin\\shpbymargin\\shpwr0\\shpwrk0\\shpfblwtxt1\\shpz0{{\\sp{{\\sn shapeType}}{{\\sv 1}}}}{{\\sp{{\\sn fillColor}}{{\\sv {}}}}}{{\\sp{{\\sn fFilled}}{{\\sv 1}}}}{{\\sp{{\\sn fLine}}{{\\sv 0}}}}{{\\sp{{\\sn fBackground}}{{\\sv 1}}}}}}}}}}",
                bgr(color)
            );
        }
        self.out.push('\n');
    }

    fn stylesheet(&mut self) {
        let document = self.document;
        let styles = &document.styles;
        // The default paragraph style is number 0; the rest follow in order.
        let mut next = 1;
        self.paragraph_numbers = (0..styles.paragraph.len())
            .map(|index| {
                if Some(index) == styles.default_paragraph {
                    0
                } else {
                    let number = next;
                    next += 1;
                    number
                }
            })
            .collect();
        self.character_base = next + 10;
        let mut sheet = String::from("{\\stylesheet");
        if styles.default_paragraph.is_none() {
            sheet.push_str("{\\s0\\ql Normal;}");
        }
        for (index, style) in styles.paragraph.iter().enumerate() {
            let number = self.paragraph_numbers[index];
            let _ = write!(sheet, "{{\\s{number}");
            if let Some(parent) = style.parent {
                let _ = write!(sheet, "\\sbasedon{}", self.paragraph_numbers[parent]);
            }
            let paragraph = document.paragraph_style_properties(index);
            let run = document.paragraph_style_run(index);
            let mut properties = String::new();
            self.paragraph_properties(&paragraph, &mut properties);
            self.run_properties(&run, &mut properties);
            sheet.push_str(&properties);
            sheet.push(' ');
            escape_into(&mut sheet, &style.name);
            sheet.push_str(";}");
        }
        for (index, style) in styles.character.iter().enumerate() {
            let number = self.character_base + index;
            let _ = write!(sheet, "{{\\*\\cs{number}\\additive");
            if let Some(parent) = style.parent {
                let _ = write!(sheet, "\\sbasedon{}", self.character_base + parent);
            }
            let run = document.character_style_run(index);
            let mut properties = String::new();
            self.run_properties(&run, &mut properties);
            sheet.push_str(&properties);
            sheet.push(' ');
            escape_into(&mut sheet, &style.name);
            sheet.push_str(";}");
        }
        sheet.push_str("}\n");
        self.out.push_str(&sheet);
    }

    fn list_tables(&mut self) {
        let document = self.document;
        if document.styles.list.is_empty() {
            return;
        }
        let symbol = self.font("Symbol");
        let wingdings = self.font("Wingdings");
        let mut table = String::from("{\\*\\listtable");
        for (index, style) in document.styles.list.iter().enumerate() {
            let _ = write!(table, "{{\\list\\listtemplateid{}\\listhybrid", index + 1);
            for level in 0..9 {
                let definition = style
                    .levels
                    .get(level)
                    .or_else(|| style.levels.last())
                    .cloned()
                    .unwrap_or_default();
                let (format, text, numbers, font) = match &definition.label {
                    ListLabel::None => (255, Vec::new(), Vec::new(), None),
                    ListLabel::Text(marker) => {
                        let (font, marker) = match marker.trim() {
                            "\u{2022}" | "\u{25CF}" => (Some(symbol), "\u{F0B7}".to_string()),
                            "\u{25AA}" | "\u{25A0}" => (Some(wingdings), "\u{F0A7}".to_string()),
                            _ => (None, marker.clone()),
                        };
                        let units: Vec<u16> = marker.encode_utf16().collect();
                        (23, units, Vec::new(), font)
                    }
                    ListLabel::Number(number) => {
                        let format = match number.kind {
                            NumberKind::Decimal => 0,
                            NumberKind::UpperRoman => 1,
                            NumberKind::LowerRoman => 2,
                            NumberKind::UpperLetter => 3,
                            NumberKind::LowerLetter => 4,
                        };
                        let mut units: Vec<u16> = Vec::new();
                        let mut numbers: Vec<usize> = Vec::new();
                        if number.tiered {
                            for parent in 0..level {
                                numbers.push(units.len() + 1);
                                units.push(parent as u16);
                                units.push(u16::from(b'.'));
                            }
                        }
                        let mut rest = number.pattern.as_str();
                        while let Some(at) = rest.find("%1") {
                            units.extend(rest[..at].encode_utf16());
                            numbers.push(units.len() + 1);
                            units.push(level as u16);
                            rest = &rest[at + 2..];
                        }
                        units.extend(rest.encode_utf16());
                        (format, units, numbers, None)
                    }
                };
                let left = twips(definition.indent);
                let first = twips(definition.label_indent - definition.indent);
                let _ = write!(
                    table,
                    "{{\\listlevel\\levelnfc{format}\\levelnfcn{format}\\leveljc0\\leveljcn0\\levelfollow0\\levelstartat1\\levelspace0\\levelindent0{{\\leveltext\\'{:02x}",
                    text.len().min(255)
                );
                for unit in &text {
                    if *unit < 0x20 {
                        let _ = write!(table, "\\'{:02x}", unit);
                    } else {
                        escape_unit(&mut table, *unit);
                    }
                }
                table.push_str(";}{\\levelnumbers");
                for position in &numbers {
                    let _ = write!(table, "\\'{:02x}", position);
                }
                table.push_str(";}");
                if let Some(font) = font {
                    let _ = write!(table, "\\f{font}");
                }
                let _ = write!(table, "\\fi{first}\\li{left}\\lin{left}}}");
            }
            let _ = write!(table, "{{\\listname ;}}\\listid{}}}", index + 1);
        }
        table.push_str("}\n{\\*\\listoverridetable");
        for (index, entry) in self.list_overrides.iter().enumerate() {
            match entry.start {
                Some(start) => {
                    let _ = write!(
                        table,
                        "{{\\listoverride\\listid{}\\listoverridecount1{{\\lfolevel\\listoverridestartat\\levelstartat{start}}}\\ls{}}}",
                        entry.style + 1,
                        index + 1
                    );
                }
                None => {
                    let _ = write!(
                        table,
                        "{{\\listoverride\\listid{}\\listoverridecount0\\ls{}}}",
                        entry.style + 1,
                        index + 1
                    );
                }
            }
        }
        table.push_str("}\n");
        self.out.push_str(&table);
    }

    // ----- sections -----

    fn section(&mut self, section: &Section, first: bool) {
        let page = &section.page;
        let _ = write!(
            self.out,
            "\\sectd\\pgwsxn{}\\pghsxn{}\\marglsxn{}\\margrsxn{}\\margtsxn{}\\margbsxn{}\\headery{}\\footery{}",
            twips(page.width),
            twips(page.height),
            twips(page.margin_left),
            twips(page.margin_right),
            twips(page.margin_top),
            twips(page.margin_bottom),
            twips(page.header_distance),
            twips(page.footer_distance)
        );
        if page.width > page.height {
            self.out.push_str("\\lndscpsxn");
        }
        if !first {
            self.out.push_str(match section.start {
                SectionStart::Continuous => "\\sbknone",
                SectionStart::NewPage => "\\sbkpage",
            });
        }
        if let Some(start) = page.page_number_start {
            let _ = write!(self.out, "\\pgnrestart\\pgnstarts{start}");
        }
        if !section.column_widths.is_empty() {
            let _ = write!(self.out, "\\cols{}", section.column_widths.len());
            for (index, (width, gap)) in section.column_widths.iter().enumerate() {
                let _ = write!(self.out, "\\colno{}\\colw{}", index + 1, twips(*width));
                if index + 1 < section.column_widths.len() {
                    let _ = write!(self.out, "\\colsr{}", twips(*gap));
                }
            }
        } else if section.columns > 1 {
            let _ = write!(self.out, "\\cols{}", section.columns);
            if let Some(gap) = section.column_gap {
                let _ = write!(self.out, "\\colsx{}", twips(gap));
            }
        }
        if section.headers.first.is_some() || section.footers.first.is_some() {
            self.out.push_str("\\titlepg");
        }
        self.out.push('\n');
        self.page_part(&section.headers, "header");
        self.page_part(&section.footers, "footer");
    }

    fn page_part(&mut self, variants: &PageVariants, kind: &str) {
        let parts: [(&Option<Vec<Block>>, &str); 3] = if variants.even.is_some() {
            [
                (&variants.default, "r"),
                (&variants.even, "l"),
                (&variants.first, "f"),
            ]
        } else {
            [(&variants.default, ""), (&None, ""), (&variants.first, "f")]
        };
        for (blocks, suffix) in parts {
            let Some(blocks) = blocks else {
                continue;
            };
            let _ = write!(self.out, "{{\\{kind}{suffix}\\pard\\plain ");
            let _ = self.blocks(blocks, Place { depth: 0 }, false);
            self.out.push_str("}\n");
        }
    }

    /// Floating objects placed on a page rather than in the text: written
    /// at the start of the first paragraph, positioned on the page.
    fn unplaced_floating(&mut self) {
        let document = self.document;
        let mut anchored = vec![false; document.floating.len()];
        let mut paragraphs: Vec<&Paragraph> = Vec::new();
        for section in &document.sections {
            collect_paragraphs(&section.blocks, &mut paragraphs);
        }
        for paragraph in paragraphs {
            for run in &paragraph.runs {
                if let Inline::Anchor(id) = run.content
                    && let Some(slot) = anchored.get_mut(id as usize)
                {
                    *slot = true;
                }
            }
        }
        let loose: Vec<usize> = (0..document.floating.len())
            .filter(|index| !anchored[*index] && document.floating[*index].repeats.is_none())
            .collect();
        if loose.is_empty() {
            return;
        }
        self.out.push_str("\\pard\\plain ");
        for index in loose {
            self.shape(index as Id);
        }
    }

    // ----- blocks -----

    /// Writes blocks; `last_ends` says whether the final paragraph ends with
    /// `\par` (a story's last paragraph ends with its group in footnotes and
    /// headers).
    fn blocks(&mut self, blocks: &[Block], place: Place, last_ends: bool) -> io::Result<()> {
        for (index, block) in blocks.iter().enumerate() {
            let last = index + 1 == blocks.len();
            match block {
                Block::Paragraph(paragraph) => {
                    // LibreOffice loses the page break of a section whose
                    // last paragraph is a picture ended by `\sect`.
                    let picture_last = self.section_end
                        && last
                        && matches!(
                            paragraph.runs.last().map(|run| run.content),
                            Some(Inline::Image(_) | Inline::Anchor(_))
                        );
                    let end = if !last || last_ends || picture_last {
                        "\\par"
                    } else {
                        ""
                    };
                    self.paragraph(paragraph, place, end);
                }
                Block::Table(table) => self.table(table, place),
            }
            self.flush_if_large()?;
        }
        Ok(())
    }

    fn paragraph(&mut self, paragraph: &Paragraph, place: Place, end: &str) {
        let document = self.document;
        let mut head = String::from("\\pard\\plain");
        if let Some(style) = paragraph.style {
            let _ = write!(head, "\\s{}", self.paragraph_numbers[style]);
        }
        if place.depth > 0 {
            head.push_str("\\intbl");
            if place.depth > 1 {
                let _ = write!(head, "\\itap{}", place.depth);
            }
        }
        let properties = document.effective_paragraph(paragraph);
        self.paragraph_properties(&properties, &mut head);
        let mut label = None;
        if let Some(item) = paragraph.list {
            let instance = self
                .instance_of
                .get(&(paragraph as *const Paragraph as usize))
                .copied()
                .unwrap_or(item.style);
            let _ = write!(head, "\\ls{}\\ilvl{}", instance + 1, item.level);
            label = self.list_label(instance, item);
        }
        // The head ends with a control word: a space closes it.
        self.out.push_str(&head);
        self.out.push(' ');
        if let Some(label) = label {
            self.out.push_str("{\\listtext\\pard\\plain ");
            escape_into(&mut self.out, &label);
            self.out.push_str("\\tab}");
        }
        self.runs(paragraph);
        if !end.is_empty() {
            let mark = document.paragraph_mark_properties(paragraph);
            if !mark.is_empty() {
                let mut properties = String::from("{\\plain");
                let mut effective = match paragraph.style.or(document.styles.default_paragraph) {
                    Some(style) => document.paragraph_style_run(style),
                    None => RunProperties::default(),
                };
                effective.overlay(&mark);
                self.run_properties(&effective, &mut properties);
                properties.push_str(end);
                properties.push('}');
                self.out.push_str(&properties);
            } else {
                self.out.push_str(end);
            }
            self.out.push('\n');
        }
    }

    /// The label an item shows, counted as Word counts it.
    fn list_label(&mut self, instance: usize, item: crate::document::ListItem) -> Option<String> {
        let document = self.document;
        let style = document.styles.list.get(item.style)?;
        let level = usize::from(item.level);
        let definition = style.levels.get(level).or_else(|| style.levels.last())?;
        let start = self
            .list_overrides
            .get(instance)
            .and_then(|entry| entry.start)
            .unwrap_or(1);
        let counters = self.counters.entry(instance).or_insert_with(|| vec![0; 9]);
        if counters[level] == 0 {
            counters[level] = if level == 0 { start } else { 1 };
        } else {
            counters[level] += 1;
        }
        for deeper in counters.iter_mut().skip(level + 1) {
            *deeper = 0;
        }
        let snapshot = counters.clone();
        match &definition.label {
            ListLabel::None => None,
            ListLabel::Text(marker) => Some(marker.clone()),
            ListLabel::Number(number) => {
                let mut label = String::new();
                if number.tiered {
                    for (parent, value) in snapshot.iter().enumerate().take(level) {
                        let kind = match style.levels.get(parent).map(|level| &level.label) {
                            Some(ListLabel::Number(format)) => format.kind,
                            _ => NumberKind::Decimal,
                        };
                        label.push_str(&kind.format((*value).max(1)));
                        label.push('.');
                    }
                }
                label.push_str(&number.label(snapshot[level].max(1)));
                Some(label)
            }
        }
    }

    fn runs(&mut self, paragraph: &Paragraph) {
        let document = self.document;
        let mut open_link: Option<Id> = None;
        for run in &paragraph.runs {
            if run.link != open_link {
                if open_link.is_some() {
                    self.out.push_str("}}");
                }
                open_link = run.link;
                if let Some(target) = document.link(run) {
                    self.out.push_str("{\\field{\\*\\fldinst HYPERLINK ");
                    match target.strip_prefix('#') {
                        Some(anchor) => {
                            self.out.push_str("\\\\l \"");
                            escape_into(&mut self.out, anchor);
                        }
                        None => {
                            self.out.push('"');
                            escape_into(&mut self.out, target);
                        }
                    }
                    self.out.push_str("\"}{\\fldrslt ");
                }
            }
            self.run(paragraph, run);
        }
        if open_link.is_some() {
            self.out.push_str("}}");
        }
    }

    fn run(&mut self, paragraph: &Paragraph, run: &Run) {
        let document = self.document;
        let mut properties = String::from("{");
        if let Some(style) = run.style {
            let _ = write!(properties, "\\cs{}", self.character_base + style);
        }
        let effective = document.effective_run(paragraph, run);
        self.run_properties(&effective, &mut properties);
        if let Some(revision) = document.revision(run) {
            let author = revision.author.clone().unwrap_or_default();
            let author = self
                .authors
                .iter()
                .position(|known| *known == author)
                .map_or(0, |index| index + 1);
            let date = revision.date.as_deref().and_then(packed_date);
            match revision.kind {
                RevisionKind::Insertion => {
                    let _ = write!(properties, "\\revised\\revauth{author}");
                    if let Some(date) = date {
                        let _ = write!(properties, "\\revdttm{date}");
                    }
                }
                RevisionKind::Deletion => {
                    let _ = write!(properties, "\\deleted\\revauthdel{author}");
                    if let Some(date) = date {
                        let _ = write!(properties, "\\revdttmdel{date}");
                    }
                }
            }
        }
        if properties.len() > 1 {
            // Close the last control word.
            properties.push(' ');
        }
        self.out.push_str(&properties);
        match run.content {
            Inline::Text(span) => escape_into(&mut self.out, document.text(span)),
            Inline::LineBreak => self.out.push_str("\\line "),
            Inline::Tab => self.out.push_str("\\tab "),
            Inline::PageBreak => self.out.push_str("\\page "),
            Inline::ColumnBreak => self.out.push_str("\\column "),
            Inline::Math(span) => {
                let text = crate::document::mathml_text(document.text(span));
                escape_into(&mut self.out, &text);
            }
            Inline::Footnote(note) => self.footnote(note),
            Inline::Image(image) => self.image(image),
            Inline::PageNumber => {
                self.out
                    .push_str("{\\field{\\*\\fldinst PAGE}{\\fldrslt 1}}");
            }
            Inline::PageCount => {
                self.out
                    .push_str("{\\field{\\*\\fldinst NUMPAGES}{\\fldrslt 1}}");
            }
            Inline::Anchor(id) => self.shape(id),
            Inline::CommentStart(id) => {
                let _ = write!(self.out, "{{\\*\\atrfstart {id}}}");
            }
            Inline::CommentEnd(id) => {
                let _ = write!(self.out, "{{\\*\\atrfend {id}}}");
                self.comment(id);
            }
        }
        self.out.push('}');
    }

    fn footnote(&mut self, note: usize) {
        let document = self.document;
        let Some(note) = document.footnotes.get(note) else {
            return;
        };
        self.out
            .push_str("{\\super\\chftn}{\\footnote{\\super\\chftn}");
        let _ = self.blocks(&note.blocks, Place { depth: 0 }, false);
        self.out.push('}');
    }

    /// A comment and its replies, written after its range.
    fn comment(&mut self, id: Id) {
        let document = self.document;
        let index = id as usize;
        if self.comments_written.get(index).copied().unwrap_or(true) {
            return;
        }
        self.comments_written[index] = true;
        let mut thread = vec![index];
        for (reply, comment) in document.comments.iter().enumerate() {
            if comment.reply_to == Some(id) {
                thread.push(reply);
                if let Some(slot) = self.comments_written.get_mut(reply) {
                    *slot = true;
                }
            }
        }
        for (position, entry) in thread.into_iter().enumerate() {
            let comment = &document.comments[entry];
            let initials = comment
                .initials
                .clone()
                .unwrap_or_else(|| initials_of(&comment.author));
            self.out.push_str("{\\*\\atnid ");
            escape_into(&mut self.out, &initials);
            self.out.push_str("}{\\*\\atnauthor ");
            escape_into(&mut self.out, &comment.author);
            self.out.push_str("}\\chatn{\\*\\annotation");
            if position == 0 {
                let _ = write!(self.out, "{{\\*\\atnref {id}}}");
            } else {
                let _ = write!(self.out, "{{\\*\\atnparent {id}}}");
            }
            if let Some(date) = comment.date.as_deref().and_then(packed_date) {
                let _ = write!(self.out, "{{\\*\\atndate {date}}}");
            }
            self.out.push_str("\\pard\\plain ");
            for (line, text) in comment.text.split('\n').enumerate() {
                if line > 0 {
                    self.out.push_str("\\par ");
                }
                escape_into(&mut self.out, text);
            }
            self.out.push('}');
        }
    }

    fn image(&mut self, id: Id) {
        let document = self.document;
        let Some(image) = document.image(id) else {
            return;
        };
        let Some(media) = document.media.get(image.media) else {
            return;
        };
        let mut picture = String::new();
        if picture_group(
            &mut picture,
            &media.name,
            &media.bytes,
            image.width,
            image.height,
            image.crop,
            image.description.as_deref(),
        ) {
            self.out.push_str("{\\*\\shppict");
            self.out.push_str(&picture);
            self.out.push('}');
        }
    }

    /// A floating object as a shape.
    fn shape(&mut self, id: Id) {
        let document = self.document;
        let Some(object) = document.floating.get(id as usize) else {
            return;
        };
        let left = twips(object.x);
        let top = twips(object.y);
        let right = left + twips(object.width);
        let bottom = top + twips(object.height);
        let wrap = match object.wrap {
            TextWrap::TopAndBottom => 1,
            TextWrap::None | TextWrap::Inline => 3,
            TextWrap::Around => 2,
        };
        let vertical = if object.follows_text {
            "\\shpbypara"
        } else {
            "\\shpbypage"
        };
        let mut shape = format!(
            "{{\\shp{{\\*\\shpinst\\shpleft{left}\\shptop{top}\\shpright{right}\\shpbottom{bottom}\\shpbxpage\\shpbxignore{vertical}\\shpbyignore\\shpwr{wrap}\\shpz0"
        );
        if object.behind {
            shape.push_str("\\shpfblwtxt1");
        }
        let property = |shape: &mut String, name: &str, value: &str| {
            let _ = write!(shape, "{{\\sp{{\\sn {name}}}{{\\sv {value}}}}}");
        };
        if object.follows_text {
            property(&mut shape, "posrelv", "2");
        } else {
            property(&mut shape, "posrelv", "1");
        }
        property(&mut shape, "posrelh", "1");
        match &object.content {
            FloatingContent::Image(media) => {
                property(&mut shape, "shapeType", "75");
                if let Some(media) = document.media.get(*media) {
                    let mut picture = String::new();
                    if picture_group(
                        &mut picture,
                        &media.name,
                        &media.bytes,
                        object.width,
                        object.height,
                        None,
                        None,
                    ) {
                        let _ = write!(shape, "{{\\sp{{\\sn pib}}{{\\sv {picture}}}}}");
                    }
                }
                shape.push_str("}}");
                self.out.push_str(&shape);
            }
            FloatingContent::Chart(_) => {}
            FloatingContent::TextBox {
                blocks,
                fill,
                line,
                geometry,
                ..
            } => {
                let shape_type = match geometry {
                    ShapeGeometry::RoundedRectangle => "2",
                    ShapeGeometry::Ellipse => "3",
                    ShapeGeometry::Diamond => "4",
                    ShapeGeometry::Triangle => "5",
                    ShapeGeometry::RightTriangle => "6",
                    ShapeGeometry::Line => "20",
                    ShapeGeometry::Path(_) => "0",
                    _ if !blocks.is_empty() => "202",
                    _ => "1",
                };
                property(&mut shape, "shapeType", shape_type);
                if let ShapeGeometry::Path(index) = geometry
                    && let Some(path) = document.paths.get(*index as usize)
                {
                    path_properties(&mut shape, path);
                }
                match fill {
                    Some(color) => {
                        property(&mut shape, "fFilled", "1");
                        property(&mut shape, "fillColor", &bgr(*color).to_string());
                    }
                    None => property(&mut shape, "fFilled", "0"),
                }
                match line {
                    Some(border) => {
                        property(&mut shape, "fLine", "1");
                        let width = (border.width * 12700.0).round() as i64;
                        property(&mut shape, "lineWidth", &width.to_string());
                        if let Some(color) = border.color {
                            property(&mut shape, "lineColor", &bgr(color).to_string());
                        }
                    }
                    None => property(&mut shape, "fLine", "0"),
                }
                self.out.push_str(&shape);
                if !blocks.is_empty() {
                    self.out.push_str("{\\shptxt\\pard\\plain ");
                    let _ = self.blocks(blocks, Place { depth: 0 }, false);
                    self.out.push('}');
                }
                self.out.push_str("}}");
            }
        }
    }

    // ----- tables -----

    fn table(&mut self, table: &Table, place: Place) {
        let depth = place.depth + 1;
        let row_definition = |writer: &mut Writer<'_>, row: &crate::document::Row| -> String {
            writer.row_definition(table, row)
        };
        for row in &table.rows {
            let definition = row_definition(self, row);
            if depth == 1 {
                self.out.push_str(&definition);
            }
            let mut column = 0;
            while column < row.cells.len() {
                let cell = &row.cells[column];
                let span = cell_span(row, column);
                self.cell(cell, depth);
                column += span;
            }
            if depth == 1 {
                self.out.push_str("\\row\n");
            } else {
                let _ = writeln!(
                    self.out,
                    "{{\\*\\nesttableprops{definition}\\nestrow}}{{\\nonesttables\\par}}"
                );
            }
        }
    }

    fn cell(&mut self, cell: &Cell, depth: usize) {
        let blocks: &[Block] = if cell.merge == Merge::Above || cell.blocks.is_empty() {
            &[]
        } else {
            &cell.blocks
        };
        let place = Place { depth };
        if blocks.is_empty() {
            let _ = write!(self.out, "\\pard\\plain\\intbl");
            if depth > 1 {
                let _ = write!(self.out, "\\itap{depth}");
            }
        } else {
            for (index, block) in blocks.iter().enumerate() {
                let last = index + 1 == blocks.len();
                match block {
                    Block::Paragraph(paragraph) => {
                        let end = if last { "" } else { "\\par" };
                        self.paragraph(paragraph, place, end);
                    }
                    Block::Table(table) => {
                        self.table(table, place);
                        if last {
                            // A cell must end in a paragraph.
                            let _ = write!(self.out, "\\pard\\plain\\intbl");
                            if depth > 1 {
                                let _ = write!(self.out, "\\itap{depth}");
                            }
                        }
                    }
                }
            }
        }
        if depth > 1 {
            self.out.push_str("\\nestcell\n");
        } else {
            self.out.push_str("\\cell\n");
        }
    }

    fn row_definition(&mut self, table: &Table, row: &crate::document::Row) -> String {
        let mut out = String::from("\\trowd");
        let gap = table
            .cell_margins
            .map_or(108, |margins| twips(margins.left.min(margins.right)));
        let indent = twips(table.indent.unwrap_or(0.0));
        let _ = write!(out, "\\trgaph{gap}\\trleft{indent}");
        match table.alignment {
            Some(Alignment::Center) => out.push_str("\\trqc"),
            Some(Alignment::Right) => out.push_str("\\trqr"),
            _ => {}
        }
        let row_index = table
            .rows
            .iter()
            .position(|candidate| std::ptr::eq(candidate, row))
            .unwrap_or(0);
        if (row_index as u32) < table.header_rows {
            out.push_str("\\trhdr");
        }
        if let Some(height) = row.height {
            let _ = write!(out, "\\trrh{}", twips(height));
        }
        if let Some(margins) = table.cell_margins {
            let _ = write!(
                out,
                "\\trpaddl{}\\trpaddr{}\\trpaddt{}\\trpaddb{}\\trpaddfl3\\trpaddfr3\\trpaddft3\\trpaddfb3",
                twips(margins.left),
                twips(margins.right),
                twips(margins.top),
                twips(margins.bottom)
            );
        }
        if let Some(borders) = table.borders {
            for (word, line) in [
                ("trbrdrt", borders.top),
                ("trbrdrl", borders.left),
                ("trbrdrb", borders.bottom),
                ("trbrdrr", borders.right),
                ("trbrdrh", borders.inside_horizontal),
                ("trbrdrv", borders.inside_vertical),
            ] {
                let _ = write!(out, "\\{word}");
                self.border(line, &mut out);
            }
        }
        let mut edge = indent;
        let mut column = 0;
        while column < row.cells.len() {
            let cell = &row.cells[column];
            let span = cell_span(row, column);
            let width: f32 = table.columns.iter().skip(column).take(span).sum();
            let width = if width > 0.0 {
                width
            } else {
                72.0 * span as f32
            };
            edge += twips(width);
            if cell.merge == Merge::Above {
                out.push_str("\\clvmrg");
            } else if cell.row_span > 1 {
                out.push_str("\\clvmgf");
            }
            match cell.vertical_alignment {
                Some(VerticalAlignment::Top) => out.push_str("\\clvertalt"),
                Some(VerticalAlignment::Center) => out.push_str("\\clvertalc"),
                Some(VerticalAlignment::Bottom) => out.push_str("\\clvertalb"),
                None => {}
            }
            let borders = cell.borders;
            let table_borders = table.borders.unwrap_or_default();
            for (word, own, fallback) in [
                ("clbrdrt", borders.top, table_borders.top),
                ("clbrdrl", borders.left, table_borders.left),
                ("clbrdrb", borders.bottom, table_borders.bottom),
                ("clbrdrr", borders.right, table_borders.right),
            ] {
                let line = own.unwrap_or(fallback);
                let _ = write!(out, "\\{word}");
                self.border(line, &mut out);
            }
            if let Some(color) = cell.background {
                let index = self.color(color);
                let _ = write!(out, "\\clcbpat{index}");
            }
            // Word reads left padding as `\clpadt` and top as `\clpadl`.
            for (word, flag, value) in [
                ("clpadt", "clpadft", cell.margins[2]),
                ("clpadl", "clpadfl", cell.margins[0]),
                ("clpadb", "clpadfb", cell.margins[1]),
                ("clpadr", "clpadfr", cell.margins[3]),
            ] {
                if let Some(value) = value {
                    let _ = write!(out, "\\{word}{}\\{flag}3", twips(value));
                }
            }
            let _ = write!(out, "\\cellx{edge}");
            column += span;
        }
        out
    }

    fn border(&mut self, line: Option<Border>, out: &mut String) {
        match line {
            None => out.push_str("\\brdrnone"),
            Some(border) => {
                let _ = write!(out, "\\brdrs\\brdrw{}", twips(border.width).clamp(1, 255));
                if let Some(color) = border.color {
                    let index = self.color(color);
                    let _ = write!(out, "\\brdrcf{index}");
                }
            }
        }
    }

    // ----- formatting -----

    fn paragraph_properties(&mut self, properties: &ParagraphProperties, out: &mut String) {
        match properties.alignment {
            Some(Alignment::Center) => out.push_str("\\qc"),
            Some(Alignment::Right) => out.push_str("\\qr"),
            Some(Alignment::Justify) => out.push_str("\\qj"),
            Some(Alignment::Left) => out.push_str("\\ql"),
            None => {}
        }
        let left = properties.left_indent.unwrap_or(0.0);
        if properties.left_indent.is_some() {
            let _ = write!(out, "\\li{}\\lin{}", twips(left), twips(left));
        }
        if let Some(right) = properties.right_indent {
            let _ = write!(out, "\\ri{}\\rin{}", twips(right), twips(right));
        }
        if let Some(first) = properties.first_line_indent {
            let _ = write!(out, "\\fi{}", twips(first - left));
        }
        if let Some(before) = properties.space_before {
            let _ = write!(out, "\\sb{}", twips(before));
        }
        if let Some(after) = properties.space_after {
            let _ = write!(out, "\\sa{}", twips(after));
        }
        match properties.line_spacing {
            Some(LineSpacing::Relative(factor)) => {
                let _ = write!(out, "\\sl{}\\slmult1", (factor * 240.0).round() as i64);
            }
            Some(LineSpacing::Minimum(points)) => {
                let _ = write!(out, "\\sl{}\\slmult0", twips(points));
            }
            Some(LineSpacing::Exact(points)) => {
                let _ = write!(out, "\\sl-{}\\slmult0", twips(points));
            }
            None => {}
        }
        if properties.keep_with_next == Some(true) {
            out.push_str("\\keepn");
        }
        if properties.keep_lines_together == Some(true) {
            out.push_str("\\keep");
        }
        match properties.widow_control {
            Some(true) => out.push_str("\\widctlpar"),
            Some(false) => out.push_str("\\nowidctlpar"),
            None => {}
        }
        if let Some(level) = properties.outline_level {
            let _ = write!(out, "\\outlinelevel{level}");
        }
        if properties.page_break_before == Some(true) {
            out.push_str("\\pagebb");
        }
        if properties.contextual_spacing == Some(true) {
            out.push_str("\\contextualspace");
        }
        if let Some(color) = properties.background {
            let index = self.color(color);
            let _ = write!(out, "\\cbpat{index}");
        }
        if let Some(border) = properties.border {
            for (word, on) in [
                ("brdrt", border.top),
                ("brdrb", border.bottom),
                ("brdrl", border.left),
                ("brdrr", border.right),
            ] {
                if on {
                    let _ = write!(out, "\\{word}");
                    self.border(Some(border.line), out);
                }
            }
        }
        let tabs = self.document.tab_set(properties.tabs).to_vec();
        for tab in tabs {
            match tab.alignment {
                TabAlignment::Left => {}
                TabAlignment::Center => out.push_str("\\tqc"),
                TabAlignment::Right => out.push_str("\\tqr"),
                TabAlignment::Decimal => out.push_str("\\tqdec"),
            }
            match tab.leader {
                Some('.') | Some('\u{2026}') => out.push_str("\\tldot"),
                Some('\u{b7}') => out.push_str("\\tlmdot"),
                Some('-') => out.push_str("\\tlhyph"),
                Some('_') => out.push_str("\\tlul"),
                Some('=') => out.push_str("\\tleq"),
                _ => {}
            }
            let _ = write!(out, "\\tx{}", twips(tab.position));
        }
    }

    fn run_properties(&mut self, run: &RunProperties, out: &mut String) {
        let document = self.document;
        if let Some(font) = run.font {
            let name = document.string(font).to_string();
            if !name.is_empty() {
                let index = self.font(&name);
                let _ = write!(out, "\\f{index}");
            }
        }
        if let Some(size) = run.size {
            let _ = write!(out, "\\fs{}", (size * 2.0).round() as i64);
        }
        if run.bold == Some(true) {
            out.push_str("\\b");
        }
        if run.italic == Some(true) {
            out.push_str("\\i");
        }
        if run.underline == Some(true) {
            out.push_str("\\ul");
        }
        if run.strike == Some(true) {
            out.push_str("\\strike");
        }
        if let Some(color) = run.color {
            let index = self.color(color);
            let _ = write!(out, "\\cf{index}");
        }
        if let Some(color) = run.highlight {
            let index = self.color(color);
            let _ = write!(out, "\\chcbpat{index}");
        }
        match run.baseline {
            Some(Baseline::Superscript) => out.push_str("\\super"),
            Some(Baseline::Subscript) => out.push_str("\\sub"),
            None => {}
        }
        match run.caps {
            Some(Caps::All) => out.push_str("\\caps"),
            Some(Caps::Small) => out.push_str("\\scaps"),
            None => {}
        }
        if run.hidden == Some(true) {
            out.push_str("\\v");
        }
        if let Some(shift) = run.shift {
            let half_points = (shift * 2.0).round() as i64;
            if half_points > 0 {
                let _ = write!(out, "\\up{half_points}");
            } else if half_points < 0 {
                let _ = write!(out, "\\dn{}", -half_points);
            }
        }
        if let Some(spacing) = run.letter_spacing {
            let twentieths = twips(spacing);
            // `\expnd` in quarter points for older readers, `\expndtw` exact.
            let _ = write!(out, "\\expnd{}\\expndtw{twentieths}", twentieths / 5);
        }
        if let Some(scale) = run.width_scale {
            let _ = write!(out, "\\charscalex{}", scale.round() as i64);
        }
        if let Some(language) = run.language
            && let Some(lcid) = tag_lcid(document.string(language))
        {
            let _ = write!(out, "\\lang{lcid}");
        }
    }
}

// ----- helpers -----

/// Every paragraph of `blocks` in order, those in tables and text boxes
/// included.
fn collect_paragraphs<'b>(blocks: &'b [Block], out: &mut Vec<&'b Paragraph>) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => out.push(paragraph),
            Block::Table(table) => {
                for row in &table.rows {
                    for cell in &row.cells {
                        collect_paragraphs(&cell.blocks, out);
                    }
                }
            }
        }
    }
}

/// How many grid columns the cell at `column` covers: itself and the
/// `Merge::Left` cells after it.
fn cell_span(row: &crate::document::Row, column: usize) -> usize {
    let mut span = 1;
    while row
        .cells
        .get(column + span)
        .is_some_and(|cell| cell.merge == Merge::Left)
    {
        span += 1;
    }
    span
}

/// Appends text escaped for RTF: braces and backslashes escaped, tabs and
/// line breaks as words, and everything outside ASCII as `\uN?`.
fn escape_into(out: &mut String, text: &str) {
    for character in text.chars() {
        match character {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            '\t' => out.push_str("\\tab "),
            '\n' => out.push_str("\\line "),
            '\r' => {}
            '\u{A0}' => out.push_str("\\~"),
            character if (character as u32) < 0x80 => out.push(character),
            character => {
                let mut units = [0u16; 2];
                for unit in character.encode_utf16(&mut units) {
                    escape_unit(out, *unit);
                }
            }
        }
    }
}

/// One UTF-16 unit as `\uN?` (N signed, as RTF writes it), or as itself
/// when it is plain ASCII.
fn escape_unit(out: &mut String, unit: u16) {
    if (0x20..0x80).contains(&unit) {
        let character = unit as u8 as char;
        match character {
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("\\{"),
            '}' => out.push_str("\\}"),
            character => out.push(character),
        }
        return;
    }
    let _ = write!(out, "\\u{}?", unit as i16);
}

fn initials_of(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|word| word.chars().next())
        .collect()
}

/// An ISO 8601 date as RTF's packed date: minutes, hours, day, month, and
/// years since 1900 in bit fields.
fn packed_date(iso: &str) -> Option<i32> {
    let number = |range: std::ops::Range<usize>| -> Option<u32> { iso.get(range)?.parse().ok() };
    let year = number(0..4)?;
    let month = number(5..7)?;
    let day = number(8..10)?;
    let hour = number(11..13).unwrap_or(0);
    let minute = number(14..16).unwrap_or(0);
    if !(1900..=2411).contains(&year) || month == 0 || month > 12 || day == 0 || day > 31 {
        return None;
    }
    let packed = minute | hour << 6 | day << 11 | month << 16 | (year - 1900) << 20;
    Some(packed as i32)
}

/// The Windows language id for a language tag.
fn tag_lcid(tag: &str) -> Option<u32> {
    Some(match tag {
        "ar-SA" => 1025,
        "zh-TW" => 1028,
        "cs-CZ" => 1029,
        "da-DK" => 1030,
        "de-DE" => 1031,
        "el-GR" => 1032,
        "en-US" | "en" => 1033,
        "es-ES" | "es" => 3082,
        "fi-FI" => 1035,
        "fr-FR" | "fr" => 1036,
        "he-IL" => 1037,
        "hu-HU" => 1038,
        "it-IT" | "it" => 1040,
        "ja-JP" | "ja" => 1041,
        "ko-KR" | "ko" => 1042,
        "nl-NL" | "nl" => 1043,
        "nb-NO" => 1044,
        "pl-PL" => 1045,
        "pt-BR" => 1046,
        "ru-RU" | "ru" => 1049,
        "sv-SE" => 1053,
        "tr-TR" => 1055,
        "uk-UA" => 1058,
        "zh-CN" | "zh" => 2052,
        "en-GB" => 2057,
        "pt-PT" => 2070,
        "en-AU" => 3081,
        "en-CA" => 4105,
        "de" => 1031,
        _ => return None,
    })
}

/// Writes a `\pict` group for an image file: PNG and JPEG as they are,
/// EMF and WMF as metafiles, BMP as a device-independent bitmap. Returns
/// false for kinds RTF cannot carry.
fn picture_group(
    out: &mut String,
    name: &str,
    bytes: &[u8],
    width: f32,
    height: f32,
    crop: Option<[f32; 4]>,
    description: Option<&str>,
) -> bool {
    let extension = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    let (kind, data): (&str, &[u8]) = match extension.as_str() {
        "png" => ("\\pngblip", bytes),
        "jpg" | "jpeg" => ("\\jpegblip", bytes),
        "emf" => ("\\emfblip", bytes),
        "wmf" => ("\\wmetafile8", wmf_without_header(bytes)),
        "bmp" if bytes.len() > 14 => ("\\dibitmap0", &bytes[14..]),
        _ => return false,
    };
    let (pixels_width, pixels_height) =
        pixel_size(bytes).unwrap_or(((width / 0.75) as u32, (height / 0.75) as u32));
    let goal_width = twips(width).max(1);
    let goal_height = twips(height).max(1);
    let _ = write!(
        out,
        "{{\\pict{kind}\\picw{}\\pich{}\\picwgoal{goal_width}\\pichgoal{goal_height}",
        pixels_width.max(1),
        pixels_height.max(1)
    );
    if let Some([left, top, right, bottom]) = crop {
        let _ = write!(
            out,
            "\\piccropl{}\\piccropt{}\\piccropr{}\\piccropb{}",
            (left * goal_width as f32).round() as i64,
            (top * goal_height as f32).round() as i64,
            (right * goal_width as f32).round() as i64,
            (bottom * goal_height as f32).round() as i64
        );
    }
    if let Some(description) = description.filter(|text| !text.is_empty()) {
        out.push_str("{\\*\\picprop{\\sp{\\sn wzDescription}{\\sv ");
        escape_into(out, description);
        out.push_str("}}}");
    }
    out.push('\n');
    const HEX: &[u8; 16] = b"0123456789abcdef";
    for (index, byte) in data.iter().enumerate() {
        out.push(HEX[usize::from(byte >> 4)] as char);
        out.push(HEX[usize::from(byte & 15)] as char);
        if index % 64 == 63 {
            out.push('\n');
        }
    }
    out.push('}');
    true
}

/// A custom outline as Word's shape properties: its box (`geoRight`,
/// `geoBottom`, in hundredths of a point), its points (`pVerticies`), and
/// the segments joining them (`pSegmentInfo`).
fn path_properties(shape: &mut String, path: &crate::document::ShapePath) {
    use crate::document::PathStep;
    let scale = |value: f32| (value * 100.0).round() as i64;
    let mut points: Vec<(i64, i64)> = Vec::new();
    let mut segments: Vec<u32> = Vec::new();
    for step in &path.steps {
        match step {
            PathStep::Move(x, y) => {
                points.push((scale(*x), scale(*y)));
                segments.push(0x4000);
            }
            PathStep::Line(x, y) => {
                points.push((scale(*x), scale(*y)));
                segments.push(0x0001);
            }
            PathStep::Curve(controls) => {
                for (x, y) in controls {
                    points.push((scale(*x), scale(*y)));
                }
                segments.push(0x2001);
            }
            PathStep::Close => segments.push(0x6001),
        }
    }
    segments.push(0x8000);
    let mut vertices = format!("8;{}", points.len());
    for (x, y) in &points {
        let _ = write!(vertices, ";({x},{y})");
    }
    let mut info = format!("2;{}", segments.len());
    for segment in &segments {
        let _ = write!(info, ";{segment}");
    }
    let _ = write!(
        shape,
        "{{\\sp{{\\sn geoRight}}{{\\sv {}}}}}{{\\sp{{\\sn geoBottom}}{{\\sv {}}}}}{{\\sp{{\\sn pVerticies}}{{\\sv {vertices}}}}}{{\\sp{{\\sn pSegmentInfo}}{{\\sv {info}}}}}",
        scale(path.width).max(1),
        scale(path.height).max(1)
    );
}

/// A WMF without its placeable header (RTF carries the metafile itself).
fn wmf_without_header(bytes: &[u8]) -> &[u8] {
    if bytes.starts_with(&[0xD7, 0xCD, 0xC6, 0x9A]) && bytes.len() > 22 {
        &bytes[22..]
    } else {
        bytes
    }
}

/// The pixel size of a PNG or JPEG.
fn pixel_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        let width = u32::from_be_bytes(bytes.get(16..20)?.try_into().ok()?);
        let height = u32::from_be_bytes(bytes.get(20..24)?.try_into().ok()?);
        return Some((width, height));
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        let mut at = 2;
        while at + 9 < bytes.len() {
            if bytes[at] != 0xFF {
                at += 1;
                continue;
            }
            let marker = bytes[at + 1];
            let length = usize::from(u16::from_be_bytes([bytes[at + 2], bytes[at + 3]]));
            let is_frame = matches!(marker, 0xC0..=0xCF) && !matches!(marker, 0xC4 | 0xC8 | 0xCC);
            if is_frame {
                let height = u32::from(u16::from_be_bytes([bytes[at + 5], bytes[at + 6]]));
                let width = u32::from(u16::from_be_bytes([bytes[at + 7], bytes[at + 8]]));
                return Some((width, height));
            }
            at += 2 + length;
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::rtf::read_rtf;

    fn round_trip(source: &str) -> Document {
        let document = read_rtf(source.as_bytes()).expect("read");
        let mut out = Vec::new();
        write_rtf(&document, &mut out).expect("write");
        read_rtf(&out).expect("read back")
    }

    #[test]
    fn text_formatting_and_unicode_round_trip() {
        let document = round_trip(
            "{\\rtf1\\ansi{\\fonttbl\\f0 Arial;}\\f0\\fs24 Plain {\\b bold} caf\\'e9 {\\i \\u8364?}\\par\\pard\\qc Centre\\par}",
        );
        assert_eq!(
            document.paragraph_texts(),
            vec!["Plain bold café €", "Centre"]
        );
        let Block::Paragraph(first) = &document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        assert_eq!(document.run_properties(&first.runs[1]).bold, Some(true));
    }

    #[test]
    fn tables_with_merges_round_trip() {
        let document = round_trip(concat!(
            "{\\rtf1\\ansi",
            "\\trowd\\cellx2000\\cellx4000\\cellx6000",
            "\\pard\\intbl A\\cell B\\cell C\\cell\\row",
            "\\trowd\\cellx4000\\cellx6000",
            "\\pard\\intbl Wide\\cell D\\cell\\row",
            "\\pard After\\par}"
        ));
        let Block::Table(table) = &document.sections[0].blocks[0] else {
            panic!("table");
        };
        assert_eq!(table.rows[1].cells[0].column_span, 2);
        assert_eq!(
            document.paragraph_texts(),
            vec!["A", "B", "C", "Wide", "", "D", "After"]
                .into_iter()
                .filter(|text| !text.is_empty())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn lists_restart_and_start_numbers_round_trip() {
        let document = round_trip(concat!(
            "{\\rtf1\\ansi{\\*\\listtable{\\list\\listtemplateid1",
            "{\\listlevel\\levelnfc0\\levelstartat1{\\leveltext\\'02\\'00.;}{\\levelnumbers\\'01;}\\fi-360\\li720}",
            "\\listid7}}",
            "{\\*\\listoverridetable{\\listoverride\\listid7\\listoverridecount0\\ls1}",
            "{\\listoverride\\listid7\\listoverridecount1{\\lfolevel\\listoverridestartat\\levelstartat10}\\ls2}}",
            "\\pard\\ls1\\ilvl0 One\\par Two\\par\\pard\\ls2\\ilvl0 Ten\\par}"
        ));
        let items: Vec<_> = document.sections[0]
            .blocks
            .iter()
            .filter_map(|block| match block {
                Block::Paragraph(paragraph) => paragraph.list,
                _ => None,
            })
            .collect();
        assert_eq!(items.len(), 3);
        assert!(items[0].starts_list);
        assert!(!items[1].starts_list);
        assert!(items[2].starts_list);
        assert_eq!(items[2].start, 10);
    }

    #[test]
    fn links_and_footnotes_round_trip() {
        let document = round_trip(concat!(
            "{\\rtf1\\ansi See {\\field{\\*\\fldinst HYPERLINK \"https://example.com\"}{\\fldrslt here}}",
            "{\\super\\chftn}{\\footnote\\pard{\\super\\chftn} A note.}.\\par}"
        ));
        let Block::Paragraph(paragraph) = &document.sections[0].blocks[0] else {
            panic!("paragraph");
        };
        assert!(
            paragraph
                .runs
                .iter()
                .any(|run| document.link(run) == Some("https://example.com"))
        );
        assert_eq!(document.footnotes.len(), 1);
        assert_eq!(document.paragraph_texts()[0], "See here.");
    }

    #[test]
    fn packed_dates_round_trip() {
        let packed = packed_date("2024-03-15T10:30:00Z").expect("date");
        let document = round_trip(&format!(
            "{{\\rtf1\\ansi{{\\*\\revtbl{{Unknown;}}{{Ann;}}}}{{\\revised\\revauth1\\revdttm{packed} new}}\\par}}"
        ));
        let revision = &document.revisions[0];
        assert_eq!(revision.author.as_deref(), Some("Ann"));
        assert_eq!(revision.date.as_deref(), Some("2024-03-15T10:30:00Z"));
    }
}
