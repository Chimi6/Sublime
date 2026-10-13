//! OpenDocument styles: lengths, colours, the text and paragraph
//! properties of a style, list styles, page layouts, and master pages, as
//! `styles.xml` and `content.xml` declare them (ODF 1.3, part 3, 16 and
//! 20). Properties map onto the document model's; names are the
//! escaped `style:name`s (`Heading_20_1`), shown by `style:display-name`.

use std::collections::HashMap;

use crate::document::{
    Alignment, Baseline, Border, Caps, Color, Document, LineSpacing, ListLabel, ListLevel,
    NumberFormat, NumberKind, PageSetup, ParagraphBorder, ParagraphProperties, RunProperties,
    TabAlignment, TabStop, VerticalAlignment,
};
use crate::io::xml::{XmlEvent, XmlReader};

/// An attribute list as the XML reader hands it.
pub(super) type Attributes<'a> = [(&'a str, std::borrow::Cow<'a, str>)];

pub(super) fn attribute<'e>(attributes: &'e Attributes<'_>, name: &str) -> Option<&'e str> {
    attributes
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_ref())
}

/// A length (`2.54cm`, `1in`, `12pt`, `10mm`, `1pc`, `16px`) in points.
pub(super) fn length(value: &str) -> Option<f32> {
    let value = value.trim();
    let split = value
        .find(|character: char| character.is_ascii_alphabetic() || character == '%')
        .unwrap_or(value.len());
    let number: f32 = value[..split].parse().ok()?;
    let factor = match &value[split..] {
        "pt" | "" => 1.0,
        "in" => 72.0,
        "cm" => 72.0 / 2.54,
        "mm" => 72.0 / 25.4,
        "pc" => 12.0,
        "px" => 0.75,
        _ => return None,
    };
    Some(number * factor)
}

/// A percentage (`115%`) as a share (1.15).
fn percent(value: &str) -> Option<f32> {
    value
        .trim()
        .strip_suffix('%')?
        .parse::<f32>()
        .ok()
        .map(|share| share / 100.0)
}

pub(super) fn color(value: &str) -> Option<Color> {
    let hex = value.trim().strip_prefix('#')?;
    if hex.len() != 6 {
        return None;
    }
    let number = u32::from_str_radix(hex, 16).ok()?;
    Some(Color {
        red: (number >> 16) as u8,
        green: (number >> 8) as u8,
        blue: number as u8,
    })
}

/// A border's width, `None` for no border.
fn border_width(value: &str) -> Option<f32> {
    border(value).map(|line| line.width)
}

/// A border (`0.5pt solid #000000`): `None` for `none` or unreadable.
fn border(value: &str) -> Option<Border> {
    if value.trim() == "none" {
        return None;
    }
    let mut width = None;
    let mut line_color = None;
    for part in value.split_whitespace() {
        if let Some(points) = length(part) {
            width = Some(points);
        } else if let Some(found) = color(part) {
            line_color = Some(found);
        }
    }
    Some(Border {
        width: width.unwrap_or(0.5),
        color: line_color,
    })
}

/// The text properties of a style (`style:text-properties`).
pub(super) fn text_properties(
    attributes: &Attributes<'_>,
    fonts: &HashMap<String, String>,
    document: &mut Document,
) -> RunProperties {
    let mut run = RunProperties::default();
    let font = attribute(attributes, "style:font-name")
        .and_then(|name| fonts.get(name).cloned())
        .or_else(|| attribute(attributes, "fo:font-family").map(unquote));
    if let Some(font) = font {
        run.font = Some(document.intern_string(&font));
    }
    if let Some(size) = attribute(attributes, "fo:font-size").and_then(length) {
        run.size = Some(size);
    }
    match attribute(attributes, "fo:font-weight") {
        Some("bold") | Some("600") | Some("700") | Some("800") | Some("900") => {
            run.bold = Some(true)
        }
        Some(_) => run.bold = Some(false),
        None => {}
    }
    match attribute(attributes, "fo:font-style") {
        Some("italic") | Some("oblique") => run.italic = Some(true),
        Some(_) => run.italic = Some(false),
        None => {}
    }
    if let Some(style) = attribute(attributes, "style:text-underline-style") {
        run.underline = Some(style != "none");
    }
    if let Some(style) = attribute(attributes, "style:text-line-through-style") {
        run.strike = Some(style != "none");
    }
    if let Some(value) = attribute(attributes, "fo:color").and_then(color) {
        run.color = Some(value);
    }
    match attribute(attributes, "fo:background-color") {
        Some("transparent") => {}
        Some(value) => run.highlight = color(value),
        None => {}
    }
    if let Some(position) = attribute(attributes, "style:text-position") {
        // `super 58%`, `sub 58%`, `33% 58%`: a keyword or a raise in
        // percent of the font size.
        let first = position.split_whitespace().next().unwrap_or("");
        run.baseline = match first {
            "super" => Some(Baseline::Superscript),
            "sub" => Some(Baseline::Subscript),
            _ => match percent(first) {
                Some(share) if share > 0.0 => Some(Baseline::Superscript),
                Some(share) if share < 0.0 => Some(Baseline::Subscript),
                _ => None,
            },
        };
    }
    match attribute(attributes, "fo:font-variant") {
        Some("small-caps") => run.caps = Some(Caps::Small),
        Some("normal") => run.caps = None,
        _ => {}
    }
    if attribute(attributes, "fo:text-transform") == Some("uppercase") {
        run.caps = Some(Caps::All);
    }
    if attribute(attributes, "text:display") == Some("none") {
        run.hidden = Some(true);
    }
    if let Some(spacing) = attribute(attributes, "fo:letter-spacing").and_then(length) {
        run.letter_spacing = Some(spacing);
    }
    if let Some(scale) = attribute(attributes, "style:text-scale").and_then(percent) {
        run.width_scale = Some(scale * 100.0);
    }
    let language = attribute(attributes, "fo:language").filter(|language| *language != "zxx");
    if let Some(language) = language {
        let tag = match attribute(attributes, "fo:country").filter(|country| *country != "none") {
            Some(country) => format!("{language}-{country}"),
            None => language.to_string(),
        };
        run.language = Some(document.intern_string(&tag));
    }
    run
}

/// A font family as `fo:font-family` gives it, without its quotes.
fn unquote(value: &str) -> String {
    value
        .trim()
        .trim_matches('\'')
        .trim_matches('"')
        .to_string()
}

/// The paragraph properties of a style (`style:paragraph-properties`),
/// and its tab stops when it has them (read by `paragraph_children`).
pub(super) fn paragraph_properties(attributes: &Attributes<'_>) -> ParagraphProperties {
    let mut paragraph = ParagraphProperties {
        alignment: match attribute(attributes, "fo:text-align") {
            Some("center") => Some(Alignment::Center),
            Some("end") | Some("right") => Some(Alignment::Right),
            Some("justify") => Some(Alignment::Justify),
            // `start` is stated alignment too (LibreOffice writes Word's
            // explicit left as it), though it is also the default.
            Some("start") | Some("left") => Some(Alignment::Left),
            _ => None,
        },
        ..ParagraphProperties::default()
    };
    paragraph.left_indent = attribute(attributes, "fo:margin-left").and_then(length);
    paragraph.right_indent = attribute(attributes, "fo:margin-right").and_then(length);
    // ODF's text indent is relative to the left margin; the model's first
    // line indent is from the page margin, as Pages measures it.
    if let Some(indent) = attribute(attributes, "fo:text-indent").and_then(length) {
        paragraph.first_line_indent = Some(paragraph.left_indent.unwrap_or(0.0) + indent);
    }
    paragraph.space_before = attribute(attributes, "fo:margin-top").and_then(length);
    paragraph.space_after = attribute(attributes, "fo:margin-bottom").and_then(length);
    if let Some(height) = attribute(attributes, "fo:line-height") {
        paragraph.line_spacing = match percent(height) {
            Some(share) => Some(LineSpacing::Relative(share)),
            None => length(height).map(LineSpacing::Exact),
        };
    }
    if let Some(least) = attribute(attributes, "style:line-height-at-least").and_then(length) {
        paragraph.line_spacing = Some(LineSpacing::Minimum(least));
    }
    match attribute(attributes, "fo:keep-with-next") {
        Some("always") => paragraph.keep_with_next = Some(true),
        Some("auto") => paragraph.keep_with_next = Some(false),
        _ => {}
    }
    match attribute(attributes, "fo:keep-together") {
        Some("always") => paragraph.keep_lines_together = Some(true),
        Some("auto") => paragraph.keep_lines_together = Some(false),
        _ => {}
    }
    if let Some(widows) = attribute(attributes, "fo:widows") {
        paragraph.widow_control = Some(widows != "0");
    }
    match attribute(attributes, "fo:background-color") {
        Some("transparent") | None => {}
        Some(value) => paragraph.background = color(value),
    }
    if attribute(attributes, "fo:break-before") == Some("page") {
        paragraph.page_break_before = Some(true);
    }
    if attribute(attributes, "style:contextual-spacing") == Some("true") {
        paragraph.contextual_spacing = Some(true);
    }
    paragraph.border = paragraph_border(attributes);
    paragraph
}

/// A paragraph's border: sides drawn, with the first side's line.
fn paragraph_border(attributes: &Attributes<'_>) -> Option<ParagraphBorder> {
    let all = attribute(attributes, "fo:border").and_then(border);
    let side = |name: &str| attribute(attributes, name).and_then(border).or(all);
    let (top, bottom, left, right) = (
        side("fo:border-top"),
        side("fo:border-bottom"),
        side("fo:border-left"),
        side("fo:border-right"),
    );
    let line = top.or(bottom).or(left).or(right)?;
    Some(ParagraphBorder {
        top: top.is_some(),
        bottom: bottom.is_some(),
        left: left.is_some(),
        right: right.is_some(),
        line,
    })
}

/// A paragraph style's tab stops (`style:tab-stops`), relative to its
/// left indent in ODF and from the margin in the model.
pub(super) fn tab_stops(reader: &mut XmlReader<'_>, indent: f32) -> Vec<TabStop> {
    let mut stops = Vec::new();
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name: "style:tab-stop",
                attributes,
                self_closing,
            } => {
                let position = attribute(&attributes, "style:position")
                    .and_then(length)
                    .unwrap_or(0.0);
                let alignment = match attribute(&attributes, "style:type") {
                    Some("center") => TabAlignment::Center,
                    Some("right") => TabAlignment::Right,
                    Some("char") => TabAlignment::Decimal,
                    _ => TabAlignment::Left,
                };
                let leader = attribute(&attributes, "style:leader-text")
                    .and_then(|text| text.chars().next())
                    .filter(|character| *character != ' ')
                    .or_else(|| match attribute(&attributes, "style:leader-style") {
                        Some("dotted") => Some('.'),
                        Some("dash") => Some('-'),
                        Some("solid") => Some('_'),
                        _ => None,
                    });
                stops.push(TabStop {
                    position: position + indent,
                    alignment,
                    leader,
                });
                if !self_closing {
                    skip(reader, "style:tab-stop");
                }
            }
            XmlEvent::End {
                name: "style:tab-stops",
            } => break,
            _ => {}
        }
    }
    stops
}

/// A table cell's properties (`style:table-cell-properties`).
#[derive(Clone, Default)]
pub(super) struct CellStyle {
    pub background: Option<Color>,
    pub borders: [Option<Option<Border>>; 4],
    pub vertical_alignment: Option<VerticalAlignment>,
    pub padding: [Option<f32>; 4],
}

pub(super) fn cell_properties(attributes: &Attributes<'_>) -> CellStyle {
    let all = attribute(attributes, "fo:border").map(border);
    let side = |name: &str| attribute(attributes, name).map(border).or(all);
    let padding_all = attribute(attributes, "fo:padding").and_then(length);
    let padding = |name: &str| attribute(attributes, name).and_then(length).or(padding_all);
    CellStyle {
        background: attribute(attributes, "fo:background-color")
            .filter(|value| *value != "transparent")
            .and_then(color),
        borders: [
            side("fo:border-top"),
            side("fo:border-bottom"),
            side("fo:border-left"),
            side("fo:border-right"),
        ],
        vertical_alignment: match attribute(attributes, "style:vertical-align") {
            Some("top") => Some(VerticalAlignment::Top),
            Some("middle") => Some(VerticalAlignment::Center),
            Some("bottom") => Some(VerticalAlignment::Bottom),
            _ => None,
        },
        padding: [
            padding("fo:padding-top"),
            padding("fo:padding-bottom"),
            padding("fo:padding-left"),
            padding("fo:padding-right"),
        ],
    }
}

/// One level of a list style (`text:list-level-style-number` or
/// `-bullet`), and the number it starts from.
pub(super) fn list_level(
    reader: &mut XmlReader<'_>,
    element: &str,
    attributes: &Attributes<'_>,
) -> (usize, ListLevel, u32) {
    let level = attribute(attributes, "text:level")
        .and_then(|level| level.parse::<usize>().ok())
        .unwrap_or(1)
        .saturating_sub(1);
    let start = attribute(attributes, "text:start-value")
        .and_then(|value| value.parse().ok())
        .unwrap_or(1);
    let label = if element == "text:list-level-style-bullet" {
        let bullet = attribute(attributes, "text:bullet-char").unwrap_or("•");
        ListLabel::Text(bullet.to_string())
    } else if element == "text:list-level-style-number" {
        let kind = match attribute(attributes, "style:num-format") {
            Some("a") => Some(NumberKind::LowerLetter),
            Some("A") => Some(NumberKind::UpperLetter),
            Some("i") => Some(NumberKind::LowerRoman),
            Some("I") => Some(NumberKind::UpperRoman),
            Some("") => None,
            _ => Some(NumberKind::Decimal),
        };
        match kind {
            Some(kind) => {
                let prefix = attribute(attributes, "style:num-prefix").unwrap_or("");
                let suffix = attribute(attributes, "style:num-suffix").unwrap_or("");
                let shown = attribute(attributes, "text:display-levels")
                    .and_then(|levels| levels.parse::<u32>().ok())
                    .unwrap_or(1);
                ListLabel::Number(NumberFormat {
                    kind,
                    pattern: format!("{prefix}%1{suffix}"),
                    tiered: shown > 1,
                })
            }
            None => ListLabel::None,
        }
    } else {
        ListLabel::None
    };
    let mut list_level = ListLevel {
        label,
        indent: 0.0,
        label_indent: 0.0,
    };
    // Where the label and the text sit: label alignment (LibreOffice's
    // mode) gives the text's margin and the label's offset from it.
    for event in reader.by_ref() {
        match event {
            XmlEvent::Start {
                name: "style:list-level-label-alignment",
                attributes,
                ..
            } => {
                let margin = attribute(&attributes, "fo:margin-left")
                    .and_then(length)
                    .unwrap_or(0.0);
                let offset = attribute(&attributes, "fo:text-indent")
                    .and_then(length)
                    .unwrap_or(0.0);
                list_level.indent = margin;
                list_level.label_indent = margin + offset;
            }
            XmlEvent::Start {
                name: "style:list-level-properties",
                attributes,
                ..
            } => {
                // The older mode: the label's start and its minimum width.
                if let Some(start) = attribute(&attributes, "text:space-before").and_then(length) {
                    let width = attribute(&attributes, "text:min-label-width")
                        .and_then(length)
                        .unwrap_or(0.0);
                    list_level.label_indent = start;
                    list_level.indent = start + width;
                }
            }
            XmlEvent::End { name } if name == element => break,
            _ => {}
        }
    }
    (level, list_level, start)
}

/// A page layout's page (`style:page-layout-properties`) and the heights
/// of its header and footer areas (`style:header-footer-properties`).
#[derive(Clone)]
pub(super) struct PageLayout {
    pub page: PageSetup,
    pub columns: u16,
    pub column_gap: Option<f32>,
    /// Header and footer: their minimum height plus the spacing to the
    /// body, when the layout has them.
    pub header: Option<f32>,
    pub footer: Option<f32>,
    /// The page's colour: `fo:background-color`, or LibreOffice's solid
    /// `draw:fill`.
    pub background: Option<Color>,
}

impl Default for PageLayout {
    fn default() -> Self {
        PageLayout {
            page: PageSetup::default(),
            columns: 1,
            column_gap: None,
            header: None,
            footer: None,
            background: None,
        }
    }
}

impl PageLayout {
    /// The model's page: ODF's page margins reach the header, the model's
    /// reach the body, with the header at the header distance.
    pub fn setup(&self, has_header: bool, has_footer: bool) -> PageSetup {
        let mut page = self.page.clone();
        page.header_distance = page.margin_top;
        page.footer_distance = page.margin_bottom;
        if has_header {
            page.margin_top += self.header.unwrap_or(0.0);
        }
        if has_footer {
            page.margin_bottom += self.footer.unwrap_or(0.0);
        }
        page
    }
}

pub(super) fn page_layout(reader: &mut XmlReader<'_>) -> PageLayout {
    let mut layout = PageLayout::default();
    // Whether the next header-footer properties are the footer's.
    let mut in_footer = false;
    for event in reader.by_ref() {
        match event {
            XmlEvent::Start {
                name: "style:page-layout-properties",
                attributes,
                ..
            } => {
                let page = &mut layout.page;
                let get = |name: &str| attribute(&attributes, name).and_then(length);
                if let Some(width) = get("fo:page-width") {
                    page.width = width;
                }
                if let Some(height) = get("fo:page-height") {
                    page.height = height;
                }
                let all = get("fo:margin");
                // The text starts inside a page border and its padding,
                // which ODF counts apart from the margin and the model
                // (as Word) within it.
                let padding_all = get("fo:padding");
                let border_all = attribute(&attributes, "fo:border").and_then(border_width);
                let inside = |side: &str| {
                    get(&format!("fo:padding-{side}"))
                        .or(padding_all)
                        .unwrap_or(0.0)
                        + attribute(&attributes, &format!("fo:border-{side}"))
                            .and_then(border_width)
                            .or(border_all)
                            .unwrap_or(0.0)
                };
                page.margin_top =
                    get("fo:margin-top").or(all).unwrap_or(page.margin_top) + inside("top");
                page.margin_bottom = get("fo:margin-bottom")
                    .or(all)
                    .unwrap_or(page.margin_bottom)
                    + inside("bottom");
                page.margin_left =
                    get("fo:margin-left").or(all).unwrap_or(page.margin_left) + inside("left");
                page.margin_right =
                    get("fo:margin-right").or(all).unwrap_or(page.margin_right) + inside("right");
                if let Some(first) = attribute(&attributes, "style:first-page-number")
                    .and_then(|value| value.parse().ok())
                {
                    page.page_number_start = Some(first);
                }
                let fill = match attribute(&attributes, "draw:fill") {
                    Some("solid") => attribute(&attributes, "draw:fill-color"),
                    _ => None,
                };
                layout.background = fill
                    .or_else(|| attribute(&attributes, "fo:background-color"))
                    .and_then(color);
            }
            XmlEvent::Start {
                name: "style:columns",
                attributes,
                ..
            } => {
                layout.columns = attribute(&attributes, "fo:column-count")
                    .and_then(|count| count.parse().ok())
                    .unwrap_or(1)
                    .max(1);
                layout.column_gap = attribute(&attributes, "fo:column-gap").and_then(length);
            }
            XmlEvent::Start {
                name: "style:header-style",
                ..
            } => in_footer = false,
            XmlEvent::Start {
                name: "style:footer-style",
                ..
            } => in_footer = true,
            XmlEvent::Start {
                name: "style:header-footer-properties",
                attributes,
                ..
            } => {
                let height = attribute(&attributes, "fo:min-height")
                    .or_else(|| attribute(&attributes, "svg:height"))
                    .and_then(length)
                    .unwrap_or(0.0);
                let gap = if in_footer {
                    attribute(&attributes, "fo:margin-top")
                } else {
                    attribute(&attributes, "fo:margin-bottom")
                }
                .and_then(length)
                .unwrap_or(0.0);
                if in_footer {
                    layout.footer = Some(height + gap);
                } else {
                    layout.header = Some(height + gap);
                }
            }
            XmlEvent::End {
                name: "style:page-layout",
            } => break,
            _ => {}
        }
    }
    layout
}

/// Skips to the end of the element `name` (its start just read).
pub(super) fn skip(reader: &mut XmlReader<'_>, name: &str) {
    let mut depth = 1usize;
    for event in reader.by_ref() {
        match event {
            XmlEvent::Start {
                name: inner,
                self_closing: false,
                ..
            } if inner == name => depth += 1,
            XmlEvent::End { name: inner } if inner == name => {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
            _ => {}
        }
    }
}

/// A style name as people see it: the display name, else the escaped
/// name unescaped (`Heading_20_1` is `Heading 1`).
pub(super) fn display_name(attributes: &Attributes<'_>, name: &str) -> String {
    if let Some(shown) = attribute(attributes, "style:display-name") {
        return shown.to_string();
    }
    unescape_name(name)
}

/// An ODF style name's `_xx_` escapes (hex code points) undone.
pub(super) fn unescape_name(name: &str) -> String {
    let mut out = String::new();
    let mut rest = name;
    while let Some(start) = rest.find('_') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('_') {
            Some(end)
                if end > 0
                    && end <= 6
                    && after[..end]
                        .chars()
                        .all(|character| character.is_ascii_hexdigit()) =>
            {
                match u32::from_str_radix(&after[..end], 16)
                    .ok()
                    .and_then(char::from_u32)
                {
                    Some(character) => out.push(character),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('_');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_in_every_unit() {
        assert_eq!(length("1in"), Some(72.0));
        assert_eq!(length("12pt"), Some(12.0));
        assert!((length("2.54cm").unwrap() - 72.0).abs() < 0.01);
        assert!((length("25.4mm").unwrap() - 72.0).abs() < 0.01);
        assert_eq!(length("-0.5in"), Some(-36.0));
        assert_eq!(length("115%"), None);
    }

    #[test]
    fn style_names_unescape() {
        assert_eq!(unescape_name("Heading_20_1"), "Heading 1");
        assert_eq!(unescape_name("List_20_Paragraph"), "List Paragraph");
        assert_eq!(unescape_name("snake_case_name"), "snake_case_name");
        assert_eq!(unescape_name("Quote_5f_Char"), "Quote_Char");
    }
}
