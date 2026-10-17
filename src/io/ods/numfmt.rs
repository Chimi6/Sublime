//! Number formats between OpenDocument data styles and Excel format codes.
//!
//! A data style (`number:number-style`, `number:date-style`, and the rest)
//! and an Excel code (`#,##0.00;[Red]-#,##0.00`) say the same things in
//! two notations, so both are read into one model, `DataStyle`, and written
//! back out of it: an ODS cell's format reaches Excel and Numbers, and an
//! Excel cell's format reaches ODS. The model also shows a value as the
//! format does (`DataStyle::render`), for the text an ODS cell carries
//! beside its value.
//!
//! A style is sections tried in order: each but the last has a condition
//! (an ODS `style:map`, an Excel section), and the last is the default.
//! When there is more than one section the value is shown without its
//! sign, which the sections spell out themselves (`-`, parentheses), as in
//! both LibreOffice and Excel.

use std::fmt::Write as _;

use crate::io::xml::{XmlEvent, XmlReader, escape_attribute, escape_text};

/// What a style formats, by the ODS element that holds it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Number,
    Percentage,
    Currency,
    Date,
    Time,
    Boolean,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Less,
    LessEqual,
    Greater,
    GreaterEqual,
    Equal,
    NotEqual,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Part {
    Text(String),
    /// Digits: `decimals` `None` is General (as many as the value needs).
    Number {
        decimals: Option<u8>,
        min_decimals: u8,
        min_integer: u8,
        /// Of those, the leftmost that show a blank, not a zero (`?`).
        blank: u8,
        grouping: bool,
        /// Thousands the value is divided by (`0,` is one).
        scale: u8,
        /// Text set among the integer digits, by its place counted from the
        /// decimal point leftwards.
        embedded: Vec<(u8, String)>,
    },
    Scientific {
        decimals: u8,
        /// Decimals always shown; the rest only when not trailing zeros.
        min_decimals: u8,
        min_integer: u8,
        exponent_digits: u8,
        /// The exponent's multiple (engineering notation is 3).
        interval: u8,
        /// Whether a positive exponent shows its `+`.
        forced_sign: bool,
    },
    Fraction {
        /// The whole part's digits, `None` for an improper fraction.
        min_integer: Option<u8>,
        numerator: u8,
        denominator: u8,
        /// A fixed denominator (`/16`).
        fixed: Option<u32>,
    },
    Currency(String),
    Day(bool),
    Month {
        long: bool,
        textual: bool,
    },
    Year(bool),
    DayOfWeek(bool),
    Hours(bool),
    Minutes(bool),
    Seconds {
        long: bool,
        decimals: u8,
    },
    AmPm,
    Boolean,
    TextContent,
    Fill(char),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Section {
    pub kind: Kind,
    pub parts: Vec<Part>,
    /// `#rrggbb`.
    pub color: Option<String>,
    pub condition: Option<(Op, f64)>,
    /// Hours past a day (`[h]`, ODS `truncate-on-overflow="false"`).
    pub elapsed: bool,
    /// Whether a date's fields follow the reader's order
    /// (`number:automatic-order`): LibreOffice shows such a numeric date
    /// as English does, month first, with slashes.
    pub automatic_order: bool,
    /// Whether the style names its own language (`number:language`),
    /// which LibreOffice shows it in, rather than the reader's.
    pub locale: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DataStyle {
    pub sections: Vec<Section>,
}

impl DataStyle {
    /// The kind of value the style shows: its default section's.
    pub fn kind(&self) -> Kind {
        self.sections.last().map_or(Kind::Number, |section| {
            // A text section beside number sections leaves the numbers'.
            if section.kind == Kind::Text && self.sections.len() > 1 {
                self.sections[0].kind
            } else {
                section.kind
            }
        })
    }

    /// A string as a text format shows it (`_-@_-` pads it with spaces):
    /// `None` when the style has no text section.
    pub fn render_text(&self, text: &str) -> Option<String> {
        let section = self.sections.last()?;
        if section.kind != Kind::Text || !section.parts.contains(&Part::TextContent) {
            return None;
        }
        let mut out = String::new();
        for part in &section.parts {
            match part {
                Part::Text(literal) => out.push_str(literal),
                Part::TextContent => out.push_str(text),
                _ => {}
            }
        }
        Some(out)
    }

    /// A date style that is one of the document language's built-in
    /// formats becomes English's built-in of the same kind, as LibreOffice
    /// shows it to an English reader (`DD.MM.YY` in a German document is
    /// `MM/DD/YY`).
    pub fn localize(&mut self, language: &str, country: &str) {
        if language.is_empty() || (language == "en" && country == "US") {
            return;
        }
        let Some((_, _, codes)) = super::builtin::DATE_FORMATS
            .iter()
            .find(|(known, place, _)| *known == language && *place == country)
            .or_else(|| {
                super::builtin::DATE_FORMATS
                    .iter()
                    .find(|(known, _, _)| *known == language)
            })
        else {
            return;
        };
        for section in &mut self.sections {
            if section.kind != Kind::Date || section.locale.is_some() || section.automatic_order {
                continue;
            }
            let Some(code) = lo_code(section) else {
                continue;
            };
            // A code listed twice is the later index's (Czech's
            // `DD.MM.YYYY`, 20 and 21, shows as 21).
            let Some(index) = codes.iter().rposition(|known| *known == code) else {
                continue;
            };
            if let Some(english) = from_excel(ENGLISH_DATES[index]) {
                if let Some(first) = english.sections.into_iter().next() {
                    section.parts = first.parts;
                }
            }
        }
    }

    /// Whether the style names its own language.
    pub fn has_locale(&self) -> bool {
        self.sections
            .last()
            .is_some_and(|section| section.locale.is_some())
    }

    /// Whether this is plain General, which needs no style at all.
    pub fn is_general(&self) -> bool {
        matches!(self.sections.as_slice(), [section] if section.color.is_none()
            && matches!(section.parts.as_slice(), [Part::Number { decimals: None, scale: 0, embedded, .. }] if embedded.is_empty()))
    }
}

/// English's built-in dates (LibreOffice's format indices 18 to 30) as
/// Excel codes.
const ENGLISH_DATES: [&str; 13] = [
    "m/d/yy",
    "dddd, mmmm d, yyyy",
    "mm/dd/yy",
    "mm/dd/yyyy",
    "mmm d, yy",
    "mmm d, yyyy",
    "d. mmm. yyyy",
    "mmmm d, yyyy",
    "d. mmmm yyyy",
    "ddd, mmm d, yy",
    "ddd dd/mmm yy",
    "ddd, mmmm d, yyyy",
    "dddd, mmmm d, yyyy",
];

/// A date section as a LibreOffice format code in English keywords
/// (`D. MMMM YYYY`), to compare with the built-in ones; `None` when it
/// holds more than a date.
fn lo_code(section: &Section) -> Option<String> {
    let mut out = String::new();
    let mut parts = section.parts.iter().peekable();
    while let Some(part) = parts.next() {
        match part {
            Part::Day(long) => out.push_str(if *long { "DD" } else { "D" }),
            Part::Month { long, textual } => out.push_str(match (textual, long) {
                (true, true) => "MMMM",
                (true, false) => "MMM",
                (false, true) => "MM",
                (false, false) => "M",
            }),
            Part::Year(long) => out.push_str(if *long { "YYYY" } else { "YY" }),
            Part::DayOfWeek(false) => out.push_str("NN"),
            Part::DayOfWeek(true) => {
                // `NNNN` is the long day with the language's ", ".
                if let Some(Part::Text(text)) = parts.peek() {
                    if let Some(rest) = text.strip_prefix(", ") {
                        out.push_str("NNNN");
                        out.push_str(rest);
                        parts.next();
                        continue;
                    }
                }
                out.push_str("NNN");
            }
            Part::Text(text) => {
                if text.chars().any(char::is_alphabetic) {
                    out.push('"');
                    out.push_str(text);
                    out.push('"');
                } else {
                    out.push_str(text);
                }
            }
            _ => return None,
        }
    }
    Some(out)
}

/// Windows locale ids Excel tags formats with (`[$€-407]`), and the
/// language and country ODS names them by.
const LOCALES: [(u32, &str, &str); 52] = [
    (0x401, "ar", "SA"),
    (0x402, "bg", "BG"),
    (0x403, "ca", "ES"),
    (0x404, "zh", "TW"),
    (0x405, "cs", "CZ"),
    (0x406, "da", "DK"),
    (0x407, "de", "DE"),
    (0x408, "el", "GR"),
    (0x409, "en", "US"),
    (0x40A, "es", "ES"),
    (0x40B, "fi", "FI"),
    (0x40C, "fr", "FR"),
    (0x40D, "he", "IL"),
    (0x40E, "hu", "HU"),
    (0x40F, "is", "IS"),
    (0x410, "it", "IT"),
    (0x411, "ja", "JP"),
    (0x412, "ko", "KR"),
    (0x413, "nl", "NL"),
    (0x414, "nb", "NO"),
    (0x415, "pl", "PL"),
    (0x416, "pt", "BR"),
    (0x418, "ro", "RO"),
    (0x419, "ru", "RU"),
    (0x41A, "hr", "HR"),
    (0x41B, "sk", "SK"),
    (0x41D, "sv", "SE"),
    (0x41E, "th", "TH"),
    (0x41F, "tr", "TR"),
    (0x422, "uk", "UA"),
    (0x424, "sl", "SI"),
    (0x425, "et", "EE"),
    (0x426, "lv", "LV"),
    (0x427, "lt", "LT"),
    (0x439, "hi", "IN"),
    (0x804, "zh", "CN"),
    (0x807, "de", "CH"),
    (0x809, "en", "GB"),
    (0x80A, "es", "MX"),
    (0x80C, "fr", "BE"),
    (0x810, "it", "CH"),
    (0x813, "nl", "BE"),
    (0x816, "pt", "PT"),
    (0xC07, "de", "AT"),
    (0xC09, "en", "AU"),
    (0xC0A, "es", "ES"),
    (0xC0C, "fr", "CA"),
    (0x1009, "en", "CA"),
    (0x100C, "fr", "CH"),
    (0x1409, "en", "NZ"),
    (0x1809, "en", "IE"),
    (0x1C09, "en", "ZA"),
];

const COLORS: [(&str, &str); 8] = [
    ("black", "#000000"),
    ("blue", "#0000ff"),
    ("cyan", "#00ffff"),
    ("green", "#00ff00"),
    ("magenta", "#ff00ff"),
    ("red", "#ff0000"),
    ("white", "#ffffff"),
    ("yellow", "#ffff00"),
];

/// Excel's color name for an ODS color, when it has one (LibreOffice's
/// red is `#ff0000`, older files use `#c9211e` and the like for theirs).
fn color_name(color: &str) -> Option<&'static str> {
    let lower = color.to_ascii_lowercase();
    if let Some((name, _)) = COLORS.iter().find(|(_, hex)| *hex == lower) {
        return Some(name);
    }
    let channel = |at: usize| u8::from_str_radix(lower.get(at..at + 2)?, 16).ok();
    let (red, green, blue) = (channel(1)?, channel(3)?, channel(5)?);
    (red >= 0x99 && green < 0x60 && blue < 0x60).then_some("red")
}

// ---------------------------------------------------------------------
// Excel codes
// ---------------------------------------------------------------------

/// Reads an Excel number format code. `None` for General and for codes
/// with nothing to show a number with.
pub fn from_excel(code: &str) -> Option<DataStyle> {
    let mut sections: Vec<Section> = Vec::new();
    for (index, text) in split_sections(code).iter().enumerate() {
        if index == 3 {
            break;
        }
        let section = excel_section(text);
        if index > 0 && section.kind == Kind::Text {
            // The text section: an ODS text style of its own, not a
            // number's.
            break;
        }
        sections.push(section);
    }
    // Implicit conditions: two sections are >= 0 and the rest, three are
    // > 0, < 0, and zero.
    let explicit = sections.iter().any(|section| section.condition.is_some());
    if !explicit {
        match sections.len() {
            2 => sections[0].condition = Some((Op::GreaterEqual, 0.0)),
            3 => {
                sections[0].condition = Some((Op::Greater, 0.0));
                sections[1].condition = Some((Op::Less, 0.0));
            }
            _ => {}
        }
    } else if let Some(last) = sections.last_mut() {
        last.condition = None;
    }
    let style = DataStyle { sections };
    (!style.sections.is_empty() && !style.is_general()).then_some(style)
}

fn split_sections(code: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut chars = code.chars();
    while let Some(character) = chars.next() {
        match character {
            '\\' if !quoted => {
                current.push(character);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '"' => {
                quoted = !quoted;
                current.push(character);
            }
            ';' if !quoted => sections.push(std::mem::take(&mut current)),
            _ => current.push(character),
        }
    }
    sections.push(current);
    sections
}

/// A run of digit placeholders and what binds them, before it is read.
fn is_number_char(character: char) -> bool {
    matches!(character, '0' | '#' | '?' | ',' | '.')
}

fn excel_section(text: &str) -> Section {
    let mut section = Section {
        kind: Kind::Number,
        parts: Vec::new(),
        color: None,
        condition: None,
        elapsed: false,
        automatic_order: false,
        locale: None,
    };
    let chars: Vec<char> = text.chars().collect();
    let mut index = 0;
    let mut percent = false;
    let mut has_date = false;
    let mut has_time = false;
    let push_text = |parts: &mut Vec<Part>, text: &str| {
        if let Some(Part::Text(last)) = parts.last_mut() {
            last.push_str(text);
        } else {
            parts.push(Part::Text(text.to_string()));
        }
    };
    while index < chars.len() {
        let character = chars[index];
        let lower = character.to_ascii_lowercase();
        match character {
            '"' => {
                let mut literal = String::new();
                index += 1;
                while index < chars.len() && chars[index] != '"' {
                    literal.push(chars[index]);
                    index += 1;
                }
                push_text(&mut section.parts, &literal);
                index += 1;
            }
            '\\' => {
                if let Some(next) = chars.get(index + 1) {
                    push_text(&mut section.parts, &next.to_string());
                }
                index += 2;
            }
            '_' => {
                push_text(&mut section.parts, " ");
                index += 2;
            }
            '*' => {
                if let Some(next) = chars.get(index + 1) {
                    section.parts.push(Part::Fill(*next));
                }
                index += 2;
            }
            '[' => {
                let end = chars[index..]
                    .iter()
                    .position(|c| *c == ']')
                    .map_or(chars.len(), |at| index + at);
                let inner: String = chars[index + 1..end.min(chars.len())].iter().collect();
                let inner_lower = inner.to_ascii_lowercase();
                if let Some(locale) = inner.strip_prefix('$') {
                    let (symbol, lcid) = locale.split_once('-').unwrap_or((locale, ""));
                    if !symbol.is_empty() {
                        section.parts.push(Part::Currency(symbol.to_string()));
                    }
                    match u32::from_str_radix(lcid, 16) {
                        // The system's long date and time.
                        Ok(0xF800 | 0xF400) => section.automatic_order = true,
                        // A symbol's locale names the symbol only; a bare
                        // `[$-407]` sets how the number shows.
                        Ok(_) if !symbol.is_empty() => {}
                        Ok(lcid) => {
                            section.locale = LOCALES.iter().find(|(id, _, _)| *id == lcid).map(
                                |(_, language, country)| {
                                    ((*language).to_string(), (*country).to_string())
                                },
                            );
                        }
                        Err(_) => {}
                    }
                } else if let Some((_, hex)) = COLORS.iter().find(|(name, _)| *name == inner_lower)
                {
                    section.color = Some((*hex).to_string());
                } else if let Some(condition) = excel_condition(&inner) {
                    section.condition = Some(condition);
                } else if !inner.is_empty() && inner_lower.chars().all(|c| "hms".contains(c)) {
                    section.elapsed = true;
                    has_time = true;
                    let long = inner.len() > 1;
                    section.parts.push(match inner_lower.as_bytes()[0] {
                        b'h' => Part::Hours(long),
                        b'm' => Part::Minutes(long),
                        _ => Part::Seconds { long, decimals: 0 },
                    });
                }
                index = end + 1;
            }
            '@' => {
                section.kind = Kind::Text;
                section.parts.push(Part::TextContent);
                index += 1;
            }
            '%' => {
                percent = true;
                push_text(&mut section.parts, "%");
                index += 1;
            }
            _ if text[text.char_indices().nth(index).map_or(0, |(at, _)| at)..]
                .to_ascii_lowercase()
                .starts_with("general") =>
            {
                section.parts.push(Part::Number {
                    decimals: None,
                    min_decimals: 0,
                    min_integer: 1,
                    blank: 0,
                    grouping: false,
                    scale: 0,
                    embedded: Vec::new(),
                });
                index += 7;
            }
            '0' | '#' | '?' => {
                let start = index;
                let mut end = index;
                // The run: placeholders and separators, a fraction's space
                // and slash, an exponent.
                let placeholder = |n: &char| matches!(n, '0' | '#' | '?');
                while end < chars.len() {
                    let c = chars[end];
                    // A fixed denominator's digits (`/16`).
                    let denominator =
                        c.is_ascii_digit() && end > start && chars[..end].contains(&'/');
                    // The space between a fraction's whole and its numerator.
                    let fraction_space = c == ' '
                        && chars.get(end + 1).is_some_and(placeholder)
                        && chars[end + 1..].iter().find(|n| !placeholder(n)) == Some(&'/');
                    if is_number_char(c) || c == '/' || denominator || fraction_space {
                        end += 1;
                    } else if matches!(c, 'E' | 'e')
                        && matches!(chars.get(end + 1), Some('+' | '-'))
                        && chars.get(end + 2).is_some_and(|n| matches!(n, '0' | '#'))
                    {
                        end += 2;
                    } else {
                        break;
                    }
                }
                // A trailing `.` or `,` that no digit follows is the number's
                // own only when it scales (`0,`) or ends the decimals (`0.`).
                let run: String = chars[start..end].iter().collect();
                section.parts.push(number_run(&run));
                index = end;
            }
            _ if "dmyhs".contains(lower) || (lower == 'a' && is_am_pm(&chars[index..])) => {
                if lower == 'a' {
                    let length = if chars[index..]
                        .iter()
                        .take(5)
                        .collect::<String>()
                        .eq_ignore_ascii_case("am/pm")
                    {
                        5
                    } else {
                        3
                    };
                    section.parts.push(Part::AmPm);
                    index += length;
                    continue;
                }
                let run = chars[index..]
                    .iter()
                    .take_while(|c| c.to_ascii_lowercase() == lower)
                    .count();
                index += run;
                match lower {
                    'd' => {
                        has_date = true;
                        section.parts.push(match run {
                            1 => Part::Day(false),
                            2 => Part::Day(true),
                            3 => Part::DayOfWeek(false),
                            _ => Part::DayOfWeek(true),
                        });
                    }
                    'm' => section.parts.push(Part::Month {
                        long: run == 2 || run >= 4,
                        textual: run >= 3,
                    }),
                    'y' => {
                        has_date = true;
                        section.parts.push(Part::Year(run > 2));
                    }
                    'h' => {
                        has_time = true;
                        section.parts.push(Part::Hours(run > 1));
                    }
                    _ => {
                        has_time = true;
                        // Fractions of a second: `ss.00`.
                        let mut decimals = 0u8;
                        if chars.get(index) == Some(&'.') {
                            let zeros =
                                chars[index + 1..].iter().take_while(|c| **c == '0').count();
                            if zeros > 0 {
                                decimals = zeros as u8;
                                index += 1 + zeros;
                            }
                        }
                        section.parts.push(Part::Seconds {
                            long: run > 1,
                            decimals,
                        });
                    }
                }
            }
            _ => {
                push_text(&mut section.parts, &character.to_string());
                index += 1;
            }
        }
    }
    resolve_minutes(&mut section.parts);
    if section
        .parts
        .iter()
        .any(|part| matches!(part, Part::Month { .. }))
    {
        has_date = true;
    }
    if section.kind != Kind::Text {
        section.kind = if has_date {
            Kind::Date
        } else if has_time {
            Kind::Time
        } else if percent {
            Kind::Percentage
        } else if section
            .parts
            .iter()
            .any(|part| matches!(part, Part::Currency(_)))
        {
            Kind::Currency
        } else {
            Kind::Number
        };
    }
    if section.kind == Kind::Currency || section.kind == Kind::Number {
        merge_embedded(&mut section.parts);
    }
    section
}

fn is_am_pm(chars: &[char]) -> bool {
    let head: String = chars.iter().take(5).collect();
    head.to_ascii_lowercase().starts_with("am/pm")
        || chars
            .iter()
            .take(3)
            .collect::<String>()
            .eq_ignore_ascii_case("a/p")
}

fn excel_condition(inner: &str) -> Option<(Op, f64)> {
    condition(inner)
}

/// A comparison and its number (`>=0.5`, `<>0`, `!=0`); two-character
/// operators are tried first.
fn condition(text: &str) -> Option<(Op, f64)> {
    const OPERATORS: [(&str, Op); 8] = [
        (">=", Op::GreaterEqual),
        ("<=", Op::LessEqual),
        ("<>", Op::NotEqual),
        ("!=", Op::NotEqual),
        ("==", Op::Equal),
        (">", Op::Greater),
        ("<", Op::Less),
        ("=", Op::Equal),
    ];
    let (op, rest) = OPERATORS
        .iter()
        .find_map(|(symbol, op)| text.strip_prefix(symbol).map(|rest| (*op, rest)))?;
    Some((op, rest.trim().parse().ok()?))
}

/// `m` is minutes after an hour or before a second, a month otherwise.
fn resolve_minutes(parts: &mut [Part]) {
    let positions: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| !matches!(part, Part::Text(_)))
        .map(|(index, _)| index)
        .collect();
    for (at, &index) in positions.iter().enumerate() {
        if let Part::Month {
            long,
            textual: false,
        } = parts[index]
        {
            let after_hour = at > 0 && matches!(parts[positions[at - 1]], Part::Hours(_));
            let before_second = positions
                .get(at + 1)
                .is_some_and(|&next| matches!(parts[next], Part::Seconds { .. }));
            if after_hour || before_second {
                parts[index] = Part::Minutes(long);
            }
        }
    }
}

/// A run of placeholders as a number, a scientific number, or a fraction.
fn number_run(run: &str) -> Part {
    let count = |text: &str, set: &[char]| text.chars().filter(|c| set.contains(c)).count() as u8;
    let placeholders = ['0', '#', '?'];
    if let Some(e) = run.find(['E', 'e']) {
        let mantissa = &run[..e];
        let exponent = &run[e + 2..];
        let (integer, fraction) = mantissa.split_once('.').unwrap_or((mantissa, ""));
        let integer_slots = count(integer, &placeholders);
        return Part::Scientific {
            decimals: count(fraction, &placeholders),
            min_decimals: count(fraction, &['0']),
            // A `?` holds a digit's place too.
            min_integer: count(integer, &['0', '?']).max(1).min(integer_slots.max(1)),
            exponent_digits: count(exponent, &['0', '#']).max(1),
            interval: if integer_slots > 1 { integer_slots } else { 1 },
            forced_sign: run.as_bytes().get(e + 1) == Some(&b'+'),
        };
    }
    if let Some((left, right)) = run.split_once('/') {
        let (whole, numerator) = match left.trim_end().rsplit_once(' ') {
            Some((whole, numerator)) => (Some(whole), numerator),
            None => (None, left.trim_end()),
        };
        let fixed = right.trim().parse::<u32>().ok().filter(|value| *value > 0);
        return Part::Fraction {
            min_integer: whole.map(|whole| count(whole, &['0', '?'])),
            numerator: count(numerator, &placeholders).max(1),
            denominator: if fixed.is_some() {
                right.trim().len() as u8
            } else {
                count(right, &placeholders).max(1)
            },
            fixed,
        };
    }
    // Commas after the last placeholder scale by thousands.
    let trimmed = run.trim_end_matches(',');
    let scale = (run.len() - trimmed.len()) as u8;
    let (integer, fraction) = trimmed.split_once('.').unwrap_or((trimmed, ""));
    let decimals = count(fraction, &placeholders);
    Part::Number {
        decimals: Some(decimals),
        min_decimals: count(fraction, &['0']),
        min_integer: count(integer, &['0', '?']),
        blank: count(integer, &['?']),
        grouping: integer.contains(','),
        scale,
        embedded: Vec::new(),
    }
}

/// Number runs split by text (`000-0000`) are one number with the text
/// embedded, as ODS writes it.
fn merge_embedded(parts: &mut Vec<Part>) {
    let numbers: Vec<usize> = parts
        .iter()
        .enumerate()
        .filter(|(_, part)| matches!(part, Part::Number { .. }))
        .map(|(index, _)| index)
        .collect();
    if numbers.len() < 2 {
        return;
    }
    let (first, last) = (numbers[0], *numbers.last().unwrap_or(&0));
    // Only text between them, and only whole digits before the last.
    let between_ok = parts[first..=last].iter().all(|part| match part {
        Part::Text(_) => true,
        Part::Number {
            decimals, scale, ..
        } => *scale == 0 && (decimals == &Some(0) || std::ptr::eq(part, &parts[last])),
        _ => false,
    });
    if !between_ok {
        return;
    }
    let Part::Number {
        decimals,
        min_decimals,
        grouping,
        scale,
        ..
    } = parts[last].clone()
    else {
        return;
    };
    let mut embedded = Vec::new();
    let mut digits_right = 0u8;
    let mut min_integer = 0u8;
    for part in parts[first..=last].iter().rev() {
        match part {
            Part::Number {
                min_integer: zeros, ..
            } => {
                // Each run's integer slots, as zeros where the code has them.
                digits_right = digits_right.saturating_add((*zeros).max(1));
                min_integer = min_integer.saturating_add((*zeros).max(1));
            }
            Part::Text(text) => embedded.push((digits_right, text.clone())),
            _ => {}
        }
    }
    embedded.reverse();
    let merged = Part::Number {
        decimals,
        min_decimals,
        min_integer,
        blank: 0,
        grouping,
        scale,
        embedded,
    };
    parts.splice(first..=last, [merged]);
}

/// Writes the style as an Excel number format code.
pub fn to_excel(style: &DataStyle) -> String {
    let mut out = String::new();
    let standard = match style.sections.as_slice() {
        [_] => true,
        [first, _] => first.condition == Some((Op::GreaterEqual, 0.0)),
        [first, second, _] => {
            first.condition == Some((Op::Greater, 0.0)) && second.condition == Some((Op::Less, 0.0))
        }
        _ => false,
    };
    for (index, section) in style.sections.iter().take(3).enumerate() {
        if index > 0 {
            out.push(';');
        }
        // A date in the reader's order is shown in English's here.
        let english = (section.automatic_order && section.locale.is_none())
            .then(|| english_order(section))
            .flatten();
        let section = english.as_ref().unwrap_or(section);
        let lcid = section.locale.as_ref().and_then(|(language, country)| {
            LOCALES
                .iter()
                .find(|(_, known_language, known_country)| {
                    known_language == language && (known_country == country || country.is_empty())
                })
                .map(|(id, _, _)| *id)
        });
        if let Some(lcid) = lcid {
            let _ = write!(out, "[$-{lcid:X}]");
        }
        if let Some(color) = &section.color {
            if let Some(name) = color_name(color) {
                let mut name = name.to_string();
                name[..1].make_ascii_uppercase();
                let _ = write!(out, "[{name}]");
            }
        }
        if !standard {
            if let Some((op, value)) = section.condition {
                let op = match op {
                    Op::Less => "<",
                    Op::LessEqual => "<=",
                    Op::Greater => ">",
                    Op::GreaterEqual => ">=",
                    Op::Equal => "=",
                    Op::NotEqual => "<>",
                };
                let _ = write!(out, "[{op}{value}]");
            }
        }
        excel_parts(&mut out, section);
    }
    out
}

fn excel_parts(out: &mut String, section: &Section) {
    for part in &section.parts {
        match part {
            Part::Text(text) => {
                for (index, piece) in text.split('%').enumerate() {
                    if index > 0 {
                        out.push('%');
                    }
                    if piece.is_empty() {
                        continue;
                    }
                    // Punctuation Excel shows as itself; a comma is a
                    // separator among digits but text among date fields.
                    let dated = matches!(section.kind, Kind::Date | Kind::Time);
                    if piece
                        .chars()
                        .all(|c| "$-+/():!^&'~{}<>= ".contains(c) || (dated && c == ','))
                    {
                        out.push_str(piece);
                    } else {
                        out.push('"');
                        out.push_str(&piece.replace('"', "\\\""));
                        out.push('"');
                    }
                }
            }
            Part::Number {
                decimals,
                min_decimals,
                min_integer,
                blank,
                grouping,
                scale,
                embedded,
            } => {
                let Some(decimals) = decimals else {
                    out.push_str("General");
                    continue;
                };
                let integer_slots = (*min_integer as usize)
                    .max(if *grouping { 4 } else { 1 })
                    .max(
                        embedded
                            .iter()
                            .map(|(at, _)| *at as usize + 1)
                            .max()
                            .unwrap_or(0),
                    );
                let mut integer = String::new();
                for slot in (0..integer_slots).rev() {
                    for (at, text) in embedded.iter().filter(|(at, _)| *at as usize == slot + 1) {
                        let _ = at;
                        integer.push('"');
                        integer.push_str(&text.replace('"', "\\\""));
                        integer.push('"');
                    }
                    integer.push(if slot >= *min_integer as usize {
                        '#'
                    } else if slot + usize::from(*blank) >= usize::from(*min_integer) {
                        '?'
                    } else {
                        '0'
                    });
                    if *grouping && slot == 3 {
                        integer.push(',');
                    }
                }
                for (_, text) in embedded.iter().filter(|(at, _)| *at == 0) {
                    integer.push('"');
                    integer.push_str(&text.replace('"', "\\\""));
                    integer.push('"');
                }
                out.push_str(&integer);
                for _ in 0..*scale {
                    out.push(',');
                }
                if *decimals > 0 {
                    out.push('.');
                    for slot in 0..*decimals {
                        out.push(if slot < *min_decimals { '0' } else { '#' });
                    }
                }
            }
            Part::Scientific {
                decimals,
                min_decimals,
                min_integer,
                exponent_digits,
                interval,
                forced_sign,
            } => {
                let slots = (*interval).max(*min_integer).max(1);
                for slot in (0..slots).rev() {
                    out.push(if slot < (*min_integer).max(1) {
                        '0'
                    } else {
                        '#'
                    });
                }
                if *decimals > 0 {
                    out.push('.');
                    for slot in 0..*decimals {
                        out.push(if slot < *min_decimals { '0' } else { '#' });
                    }
                }
                out.push_str(if *forced_sign { "E+" } else { "E-" });
                for _ in 0..(*exponent_digits).max(1) {
                    out.push('0');
                }
            }
            Part::Fraction {
                min_integer,
                numerator,
                denominator,
                fixed,
            } => {
                if let Some(zeros) = min_integer {
                    out.push_str(if *zeros > 0 { "0 " } else { "# " });
                }
                for _ in 0..(*numerator).max(1) {
                    out.push('?');
                }
                out.push('/');
                match fixed {
                    Some(value) => {
                        let _ = write!(out, "{value}");
                    }
                    None => {
                        for _ in 0..(*denominator).max(1) {
                            out.push('?');
                        }
                    }
                }
            }
            Part::Currency(symbol) => {
                let _ = write!(out, "[${symbol}]");
            }
            Part::Day(long) => out.push_str(if *long { "dd" } else { "d" }),
            Part::Month { long, textual } => out.push_str(match (textual, long) {
                (true, true) => "mmmm",
                (true, false) => "mmm",
                (false, true) => "mm",
                (false, false) => "m",
            }),
            Part::Year(long) => out.push_str(if *long { "yyyy" } else { "yy" }),
            Part::DayOfWeek(long) => out.push_str(if *long { "dddd" } else { "ddd" }),
            Part::Hours(long) => {
                let hours = if *long { "hh" } else { "h" };
                if section.elapsed {
                    let _ = write!(out, "[{hours}]");
                } else {
                    out.push_str(hours);
                }
            }
            Part::Minutes(long) => out.push_str(if *long { "mm" } else { "m" }),
            Part::Seconds { long, decimals } => {
                out.push_str(if *long { "ss" } else { "s" });
                if *decimals > 0 {
                    out.push('.');
                    for _ in 0..*decimals {
                        out.push('0');
                    }
                }
            }
            Part::AmPm => out.push_str("AM/PM"),
            Part::Boolean => out.push_str("\"TRUE\";\"TRUE\";\"FALSE\""),
            Part::TextContent => out.push('@'),
            Part::Fill(character) => {
                out.push('*');
                out.push(*character);
            }
        }
    }
}

// ---------------------------------------------------------------------
// ODS data styles
// ---------------------------------------------------------------------

/// The data styles of one styles block, by name, with their maps
/// (condition, target style name) unresolved.
#[derive(Default)]
pub struct OdsStyles {
    pub styles: std::collections::HashMap<String, (Section, Vec<Map>)>,
}

/// A style's map: the condition and the style it applies.
pub type Map = ((Op, f64), String);

impl OdsStyles {
    /// Reads every data style element between here and the end of the
    /// enclosing styles block. Call on the `Start` of a data style.
    pub fn read_style<'a>(
        &mut self,
        reader: &mut XmlReader<'a>,
        element: &'a str,
        attributes: &[(&str, std::borrow::Cow<'_, str>)],
        self_closing: bool,
    ) {
        let kind = match element {
            "number:number-style" => Kind::Number,
            "number:percentage-style" => Kind::Percentage,
            "number:currency-style" => Kind::Currency,
            "number:date-style" => Kind::Date,
            "number:time-style" => Kind::Time,
            "number:boolean-style" => Kind::Boolean,
            "number:text-style" => Kind::Text,
            _ => return,
        };
        let get = |name: &str| {
            attributes
                .iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| value.as_ref())
        };
        let Some(name) = get("style:name").map(str::to_string) else {
            return;
        };
        let mut section = Section {
            kind,
            parts: Vec::new(),
            color: None,
            condition: None,
            elapsed: get("number:truncate-on-overflow") == Some("false"),
            automatic_order: get("number:automatic-order") == Some("true")
                && get("number:format-source") == Some("language"),
            locale: (get("number:language").is_some() || get("number:country").is_some()).then(
                || {
                    (
                        get("number:language").unwrap_or("").to_string(),
                        get("number:country").unwrap_or("").to_string(),
                    )
                },
            ),
        };
        let mut maps = Vec::new();
        if self_closing {
            self.styles.insert(name, (section, maps));
            return;
        }
        // The element whose text is being read (`number:text`,
        // `number:currency-symbol`, an embedded text).
        let mut text_target: Option<String> = None;
        let mut embedded_at: Option<u8> = None;
        let mut depth = 0usize;
        for event in reader.by_ref() {
            match event {
                XmlEvent::Start {
                    name: child,
                    attributes,
                    self_closing,
                } => {
                    let get = |name: &str| {
                        attributes
                            .iter()
                            .find(|(key, _)| *key == name)
                            .map(|(_, value)| value.as_ref())
                    };
                    let long = get("number:style") == Some("long");
                    let number = |name: &str| get(name).and_then(|v| v.parse::<u8>().ok());
                    let part = match child {
                        "number:number" => {
                            let decimals = number("number:decimal-places");
                            Some(Part::Number {
                                decimals,
                                min_decimals: number("number:min-decimal-places")
                                    .or_else(|| number("loext:min-decimal-places"))
                                    .or(decimals)
                                    .unwrap_or(0),
                                min_integer: number("number:min-integer-digits").unwrap_or(0),
                                blank: number("loext:max-blank-integer-digits").unwrap_or(0),
                                grouping: get("number:grouping") == Some("true"),
                                scale: get("number:display-factor")
                                    .and_then(|factor| factor.parse::<f64>().ok())
                                    .map_or(0, |factor| {
                                        (factor.log10() / 3.0).round().clamp(0.0, 9.0) as u8
                                    }),
                                embedded: Vec::new(),
                            })
                        }
                        "number:scientific-number" => {
                            let decimals = number("number:decimal-places").unwrap_or(0);
                            Some(Part::Scientific {
                                decimals,
                                min_decimals: number("number:min-decimal-places")
                                    .or_else(|| number("loext:min-decimal-places"))
                                    .unwrap_or(decimals),
                                min_integer: number("number:min-integer-digits").unwrap_or(1),
                                exponent_digits: number("number:min-exponent-digits").unwrap_or(1),
                                interval: number("number:exponent-interval")
                                    .or_else(|| number("loext:exponent-interval"))
                                    .unwrap_or(1),
                                forced_sign: get("number:forced-exponent-sign")
                                    .or_else(|| get("loext:forced-exponent-sign"))
                                    != Some("false"),
                            })
                        }
                        "number:fraction" => Some(Part::Fraction {
                            min_integer: number("number:min-integer-digits"),
                            numerator: number("number:min-numerator-digits").unwrap_or(1),
                            denominator: number("number:min-denominator-digits")
                                .or_else(|| number("loext:max-denominator-value").map(|_| 1))
                                .unwrap_or(1),
                            fixed: get("number:denominator-value")
                                .and_then(|value| value.parse::<u32>().ok())
                                .filter(|value| *value > 0),
                        }),
                        "number:day" => Some(Part::Day(long)),
                        "number:month" => Some(Part::Month {
                            long,
                            textual: get("number:textual") == Some("true"),
                        }),
                        "number:year" => Some(Part::Year(long)),
                        "number:day-of-week" => Some(Part::DayOfWeek(long)),
                        "number:hours" => Some(Part::Hours(long)),
                        "number:minutes" => Some(Part::Minutes(long)),
                        "number:seconds" => Some(Part::Seconds {
                            long,
                            decimals: number("number:decimal-places").unwrap_or(0),
                        }),
                        "number:am-pm" => Some(Part::AmPm),
                        "number:boolean" => Some(Part::Boolean),
                        "number:text-content" => Some(Part::TextContent),
                        "number:text" | "number:currency-symbol" | "number:fill-character" => {
                            if !self_closing {
                                text_target = Some(String::new());
                            }
                            None
                        }
                        "number:embedded-text" => {
                            embedded_at = number("number:position");
                            if !self_closing {
                                text_target = Some(String::new());
                            }
                            None
                        }
                        "style:text-properties" => {
                            if let Some(color) = get("fo:color") {
                                section.color = Some(color.to_ascii_lowercase());
                            }
                            None
                        }
                        "style:map" => {
                            if let (Some(condition), Some(target)) = (
                                get("style:condition").and_then(ods_condition),
                                get("style:apply-style-name"),
                            ) {
                                maps.push((condition, target.to_string()));
                            }
                            None
                        }
                        _ => None,
                    };
                    if let Some(part) = part {
                        section.parts.push(part);
                    }
                    if !self_closing {
                        depth += 1;
                    }
                }
                XmlEvent::Text(text) => {
                    if let Some(target) = &mut text_target {
                        target.push_str(&text);
                    }
                }
                XmlEvent::End { name: child } => {
                    if child == element && depth == 0 {
                        break;
                    }
                    depth = depth.saturating_sub(1);
                    let Some(text) = text_target.take() else {
                        continue;
                    };
                    match child {
                        "number:text" => {
                            if let Some(Part::Text(last)) = section.parts.last_mut() {
                                last.push_str(&text);
                            } else {
                                section.parts.push(Part::Text(text));
                            }
                        }
                        "number:currency-symbol" => section.parts.push(Part::Currency(text)),
                        "number:fill-character" => {
                            if let Some(character) = text.chars().next() {
                                section.parts.push(Part::Fill(character));
                            }
                        }
                        "number:embedded-text" => {
                            if let Some(Part::Number { embedded, .. }) = section
                                .parts
                                .iter_mut()
                                .rev()
                                .find(|part| matches!(part, Part::Number { .. }))
                            {
                                embedded.push((embedded_at.unwrap_or(0), text));
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        self.styles.insert(name, (section, maps));
    }

    /// The style named `name` with its maps resolved into sections.
    pub fn resolve(&self, name: &str) -> Option<DataStyle> {
        let (main, maps) = self.styles.get(name)?;
        let mut sections = Vec::with_capacity(maps.len() + 1);
        for (index, (condition, target)) in maps.iter().enumerate() {
            if let Some((section, _)) = self.styles.get(target) {
                let mut section = section.clone();
                // LibreOffice reads a second condition of `= 0` as `< 0`
                // (Excel's `[>=0.5]0;[=0]0;"[low]"` shows `[low]` for zero),
                // and shows it so.
                section.condition = Some(if index == 1 && *condition == (Op::Equal, 0.0) {
                    (Op::Less, 0.0)
                } else {
                    *condition
                });
                sections.push(section);
            }
        }
        sections.push(main.clone());
        Some(DataStyle { sections })
    }
}

/// `value()>=0` as a condition.
fn ods_condition(text: &str) -> Option<(Op, f64)> {
    condition(text.trim().strip_prefix("value()")?.trim())
}

/// Writes the style as ODS data style elements named `name` (maps' targets
/// as `{name}P0`, `{name}P1`, ...).
pub fn to_ods(out: &mut String, name: &str, style: &DataStyle) {
    let count = style.sections.len();
    for (index, section) in style.sections.iter().enumerate() {
        let is_main = index + 1 == count;
        let own = if is_main {
            name.to_string()
        } else {
            format!("{name}P{index}")
        };
        let element = match section.kind {
            Kind::Number => "number:number-style",
            Kind::Percentage => "number:percentage-style",
            Kind::Currency => "number:currency-style",
            Kind::Date => "number:date-style",
            Kind::Time => "number:time-style",
            Kind::Boolean => "number:boolean-style",
            Kind::Text => "number:text-style",
        };
        out.push('<');
        out.push_str(element);
        out.push_str(" style:name=\"");
        escape_attribute(out, &own);
        out.push('"');
        if !is_main {
            out.push_str(" style:volatile=\"true\"");
        }
        if section.elapsed {
            out.push_str(" number:truncate-on-overflow=\"false\"");
        }
        if section.automatic_order {
            out.push_str(" number:automatic-order=\"true\"");
        }
        if let Some((language, country)) = &section.locale {
            if !language.is_empty() {
                out.push_str(" number:language=\"");
                escape_attribute(out, language);
                out.push('"');
            }
            if !country.is_empty() {
                out.push_str(" number:country=\"");
                escape_attribute(out, country);
                out.push('"');
            }
        }
        out.push('>');
        if let Some(color) = &section.color {
            out.push_str("<style:text-properties fo:color=\"");
            escape_attribute(out, color);
            out.push_str("\"/>");
        }
        for part in &section.parts {
            ods_part(out, part);
        }
        if is_main {
            for (map, other) in style.sections[..count - 1].iter().enumerate() {
                if let Some((op, value)) = other.condition {
                    let op = match op {
                        Op::Less => "<",
                        Op::LessEqual => "<=",
                        Op::Greater => ">",
                        Op::GreaterEqual => ">=",
                        Op::Equal => "=",
                        Op::NotEqual => "!=",
                    };
                    out.push_str("<style:map style:condition=\"value()");
                    escape_attribute(out, op);
                    let _ = write!(out, "{value}");
                    out.push_str("\" style:apply-style-name=\"");
                    escape_attribute(out, &format!("{name}P{map}"));
                    out.push_str("\"/>");
                }
            }
        }
        out.push_str("</");
        out.push_str(element);
        out.push('>');
    }
}

fn ods_part(out: &mut String, part: &Part) {
    let long = |out: &mut String, long: bool| {
        if long {
            out.push_str(" number:style=\"long\"");
        }
    };
    match part {
        Part::Text(text) => {
            out.push_str("<number:text>");
            escape_text(out, text);
            out.push_str("</number:text>");
        }
        Part::Number {
            decimals,
            min_decimals,
            min_integer,
            blank,
            grouping,
            scale,
            embedded,
        } => {
            out.push_str("<number:number");
            if let Some(decimals) = decimals {
                let _ = write!(
                    out,
                    " number:decimal-places=\"{decimals}\" number:min-decimal-places=\"{min_decimals}\""
                );
            }
            let _ = write!(out, " number:min-integer-digits=\"{min_integer}\"");
            if *blank > 0 {
                let _ = write!(out, " loext:max-blank-integer-digits=\"{blank}\"");
            }
            if *grouping {
                out.push_str(" number:grouping=\"true\"");
            }
            if *scale > 0 {
                let _ = write!(
                    out,
                    " number:display-factor=\"{}\"",
                    10u64.pow(3 * u32::from((*scale).min(6)))
                );
            }
            if embedded.is_empty() {
                out.push_str("/>");
            } else {
                out.push('>');
                for (at, text) in embedded {
                    let _ = write!(out, "<number:embedded-text number:position=\"{at}\">");
                    escape_text(out, text);
                    out.push_str("</number:embedded-text>");
                }
                out.push_str("</number:number>");
            }
        }
        Part::Scientific {
            decimals,
            min_decimals,
            min_integer,
            exponent_digits,
            interval,
            forced_sign,
        } => {
            let _ = write!(
                out,
                "<number:scientific-number number:decimal-places=\"{decimals}\" number:min-decimal-places=\"{min_decimals}\" number:min-integer-digits=\"{min_integer}\" number:min-exponent-digits=\"{exponent_digits}\""
            );
            if *interval > 1 {
                let _ = write!(out, " number:exponent-interval=\"{interval}\"");
            }
            if !*forced_sign {
                out.push_str(" number:forced-exponent-sign=\"false\"");
            }
            out.push_str("/>");
        }
        Part::Fraction {
            min_integer,
            numerator,
            denominator,
            fixed,
        } => {
            out.push_str("<number:fraction");
            if let Some(zeros) = min_integer {
                let _ = write!(out, " number:min-integer-digits=\"{zeros}\"");
            }
            let _ = write!(
                out,
                " number:min-numerator-digits=\"{numerator}\" number:min-denominator-digits=\"{denominator}\""
            );
            if let Some(value) = fixed {
                let _ = write!(out, " number:denominator-value=\"{value}\"");
            }
            out.push_str("/>");
        }
        Part::Currency(symbol) => {
            out.push_str("<number:currency-symbol>");
            escape_text(out, symbol);
            out.push_str("</number:currency-symbol>");
        }
        Part::Day(is_long) => {
            out.push_str("<number:day");
            long(out, *is_long);
            out.push_str("/>");
        }
        Part::Month {
            long: is_long,
            textual,
        } => {
            out.push_str("<number:month");
            long(out, *is_long);
            if *textual {
                out.push_str(" number:textual=\"true\"");
            }
            out.push_str("/>");
        }
        Part::Year(is_long) => {
            out.push_str("<number:year");
            long(out, *is_long);
            out.push_str("/>");
        }
        Part::DayOfWeek(is_long) => {
            out.push_str("<number:day-of-week");
            long(out, *is_long);
            out.push_str("/>");
        }
        Part::Hours(is_long) => {
            out.push_str("<number:hours");
            long(out, *is_long);
            out.push_str("/>");
        }
        Part::Minutes(is_long) => {
            out.push_str("<number:minutes");
            long(out, *is_long);
            out.push_str("/>");
        }
        Part::Seconds {
            long: is_long,
            decimals,
        } => {
            out.push_str("<number:seconds");
            long(out, *is_long);
            if *decimals > 0 {
                let _ = write!(out, " number:decimal-places=\"{decimals}\"");
            }
            out.push_str("/>");
        }
        Part::AmPm => out.push_str("<number:am-pm/>"),
        Part::Boolean => out.push_str("<number:boolean/>"),
        Part::TextContent => out.push_str("<number:text-content/>"),
        Part::Fill(character) => {
            out.push_str("<number:fill-character>");
            escape_text(out, &character.to_string());
            out.push_str("</number:fill-character>");
        }
    }
}

// ---------------------------------------------------------------------
// Showing a value
// ---------------------------------------------------------------------

const MONTHS: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];
const WEEKDAYS: [&str; 7] = [
    "Sunday",
    "Monday",
    "Tuesday",
    "Wednesday",
    "Thursday",
    "Friday",
    "Saturday",
];

impl DataStyle {
    /// A number (a date or time as days since 1899-12-30) as the style
    /// shows it, in English.
    pub fn render(&self, value: f64) -> String {
        let Some(section) = self
            .sections
            .iter()
            .find(|section| match section.condition {
                None => true,
                Some((op, limit)) => match op {
                    Op::Less => value < limit,
                    Op::LessEqual => value <= limit,
                    Op::Greater => value > limit,
                    Op::GreaterEqual => value >= limit,
                    Op::Equal => value == limit,
                    Op::NotEqual => value != limit,
                },
            })
        else {
            return plain(value);
        };
        let signed = self.sections.len() == 1;
        render_section(section, value, signed)
    }
}

fn render_section(section: &Section, value: f64, signed: bool) -> String {
    match section.kind {
        Kind::Date | Kind::Time => {
            if section.automatic_order && section.locale.is_none() {
                if let Some(english) = english_order(section) {
                    return render_date(&english, value);
                }
            }
            return render_date(section, value);
        }
        Kind::Boolean => {
            return if value != 0.0 { "TRUE" } else { "FALSE" }.to_string();
        }
        // A number that falls to a text section shows its literal text
        // only (`0;-0;;@` hides zeros).
        Kind::Text => {
            return section
                .parts
                .iter()
                .filter_map(|part| match part {
                    Part::Text(text) => Some(text.as_str()),
                    _ => None,
                })
                .collect::<String>()
                .trim()
                .to_string();
        }
        _ => {}
    }
    let mut value = if signed { value } else { value.abs() };
    if section.kind == Kind::Percentage
        || section
            .parts
            .iter()
            .any(|part| matches!(part, Part::Text(text) if text.contains('%')))
    {
        value *= 100.0;
    }
    let mut out = String::new();
    let mut sign_pending = value < 0.0;
    let magnitude = value.abs();
    for part in &section.parts {
        let digits = match part {
            Part::Number {
                decimals,
                min_decimals,
                min_integer,
                blank,
                grouping,
                scale,
                embedded,
            } => Some(render_number(
                magnitude / 1000f64.powi(i32::from(*scale)),
                *decimals,
                *min_decimals,
                (*min_integer, *blank),
                *grouping,
                embedded,
            )),
            Part::Scientific {
                decimals,
                min_decimals,
                exponent_digits,
                interval,
                forced_sign,
                ..
            } => Some(render_scientific(
                magnitude,
                (*decimals, *min_decimals),
                *exponent_digits,
                *interval,
                *forced_sign,
            )),
            Part::Fraction {
                min_integer,
                numerator,
                denominator,
                fixed,
            } => Some(render_fraction(
                magnitude,
                *min_integer,
                *numerator,
                *denominator,
                *fixed,
            )),
            Part::Text(text) => {
                out.push_str(text);
                None
            }
            Part::Currency(symbol) => {
                out.push_str(symbol);
                None
            }
            Part::Boolean => Some(if value != 0.0 { "TRUE" } else { "FALSE" }.to_string()),
            _ => None,
        };
        if let Some(digits) = digits {
            if sign_pending && digits.chars().any(|c| c.is_ascii_digit() && c != '0') {
                out.push('-');
            }
            sign_pending = false;
            out.push_str(&digits);
        }
    }
    out
}

/// A number in LibreOffice's General format: its shortest decimal of at
/// most fifteen significant digits.
pub fn general(value: f64) -> String {
    plain(value)
}

/// A number's shortest decimal, at most fifteen significant digits.
pub fn plain(value: f64) -> String {
    if value == 0.0 {
        return "0".to_string();
    }
    let rounded = format!("{value:.14e}");
    let parsed: f64 = rounded.parse().unwrap_or(value);
    let text = format!("{parsed}");
    if text.contains('e') {
        return text;
    }
    text
}

fn render_number(
    value: f64,
    decimals: Option<u8>,
    min_decimals: u8,
    (min_integer, blank): (u8, u8),
    grouping: bool,
    embedded: &[(u8, String)],
) -> String {
    let text = match decimals {
        None => {
            // General: up to ten decimals, as LibreOffice's standard format
            // shows within fifteen significant digits.
            let general = plain(value);
            if general.contains('e') {
                return general;
            }
            general
        }
        Some(places) => format!("{:.*}", usize::from(places), round_half_away(value, places)),
    };
    let (integer, fraction) = text.split_once('.').unwrap_or((&text, ""));
    let mut fraction = fraction.to_string();
    if decimals.is_some() {
        while fraction.len() > usize::from(min_decimals) && fraction.ends_with('0') {
            fraction.pop();
        }
    }
    let mut integer = integer.trim_start_matches('0').to_string();
    let significant = integer.len();
    while integer.len() < usize::from(min_integer) {
        integer.insert(0, '0');
    }
    if integer.is_empty() && fraction.is_empty() && decimals.is_none() {
        integer.push('0');
    }
    // Embedded text among the digits, by place from the point.
    let mut integer_out = String::new();
    let digits: Vec<char> = integer.chars().collect();
    let count = digits.len();
    // Padding zeros in the blank slots (`?`) are figure spaces, and a
    // separator among them a space, as LibreOffice shows them.
    let blank_from = usize::from(min_integer.saturating_sub(blank));
    let is_blank = |place: usize| place > significant.max(blank_from);
    for (index, digit) in digits.iter().enumerate() {
        let place = count - index;
        if grouping && index > 0 && place % 3 == 0 {
            integer_out.push(if is_blank(place + 1) { ' ' } else { ',' });
        }
        if is_blank(place) {
            integer_out.push('\u{2007}');
            continue;
        }
        for (at, text) in embedded {
            if usize::from(*at) == place && index > 0 {
                integer_out.push_str(text);
            }
        }
        integer_out.push(*digit);
    }
    for (at, text) in embedded {
        if usize::from(*at) >= count && count > 0 && usize::from(*at) > 0 {
            if usize::from(*at) == count {
                integer_out.insert_str(0, text);
            }
        } else if *at == 0 {
            integer_out.push_str(text);
        }
    }
    if fraction.is_empty() {
        integer_out
    } else {
        format!("{integer_out}.{fraction}")
    }
}

fn round_half_away(value: f64, places: u8) -> f64 {
    // Through the decimal text, so 2.675 at two places is 2.68 as the
    // value's shortest decimal reads, as spreadsheets round.
    let text = plain(value);
    let Some((integer, fraction)) = text.split_once('.') else {
        return value;
    };
    if fraction.len() <= usize::from(places) || text.contains('e') {
        return value;
    }
    let keep = &fraction[..usize::from(places)];
    let next = fraction.as_bytes()[usize::from(places)];
    let base: f64 = format!("{integer}.{keep}").parse().unwrap_or(value);
    if next >= b'5' {
        let step = 10f64.powi(-i32::from(places));
        if base < 0.0 { base - step } else { base + step }
    } else {
        base
    }
}

fn render_scientific(
    value: f64,
    (decimals, min_decimals): (u8, u8),
    exponent_digits: u8,
    interval: u8,
    forced_sign: bool,
) -> String {
    let interval = i32::from(interval.max(1));
    let mut exponent = if value == 0.0 {
        0
    } else {
        value.abs().log10().floor() as i32
    };
    exponent -= exponent.rem_euclid(interval);
    let mut mantissa = value / 10f64.powi(exponent);
    let mut text = format!("{:.*}", usize::from(decimals), mantissa);
    if text.split('.').next().map_or(0, str::len) > interval as usize && value != 0.0 {
        exponent += interval;
        mantissa = value / 10f64.powi(exponent);
        text = format!("{:.*}", usize::from(decimals), mantissa);
    }
    if let Some((integer, fraction)) = text.split_once('.') {
        let mut fraction = fraction.to_string();
        while fraction.len() > usize::from(min_decimals) && fraction.ends_with('0') {
            fraction.pop();
        }
        text = if fraction.is_empty() {
            integer.to_string()
        } else {
            format!("{integer}.{fraction}")
        };
    }
    let sign = if exponent < 0 {
        "-"
    } else if forced_sign {
        "+"
    } else {
        ""
    };
    format!(
        "{text}E{sign}{:0width$}",
        exponent.abs(),
        width = usize::from(exponent_digits)
    )
}

fn render_fraction(
    value: f64,
    min_integer: Option<u8>,
    numerator_digits: u8,
    denominator_digits: u8,
    fixed: Option<u32>,
) -> String {
    let _ = numerator_digits;
    let whole = if min_integer.is_some() {
        value.trunc()
    } else {
        0.0
    };
    let rest = value - whole;
    let (numerator, denominator) = match fixed {
        Some(denominator) => (
            (rest * f64::from(denominator)).round() as i64,
            i64::from(denominator),
        ),
        None => closest_fraction(
            rest,
            10i64.pow(u32::from(denominator_digits.clamp(1, 6))) - 1,
        ),
    };
    let (whole, numerator) = if numerator == denominator && denominator > 0 {
        (whole + 1.0, 0)
    } else {
        (whole, numerator)
    };
    match min_integer {
        Some(_) if numerator == 0 => format!("{whole}"),
        Some(zeros) if whole == 0.0 && zeros == 0 => format!("{numerator}/{denominator}"),
        Some(_) => format!("{whole} {numerator}/{denominator}"),
        None => format!("{numerator}/{denominator}"),
    }
}

/// The closest fraction with a denominator at most `limit`.
fn closest_fraction(value: f64, limit: i64) -> (i64, i64) {
    let mut best = (value.round() as i64, 1i64);
    let mut error = (value - best.0 as f64).abs();
    for denominator in 2..=limit.min(10_000) {
        let numerator = (value * denominator as f64).round() as i64;
        let candidate = (value - numerator as f64 / denominator as f64).abs();
        if candidate + 1e-12 < error {
            best = (numerator, denominator);
            error = candidate;
        }
    }
    best
}

/// A numeric day, month, and year (and any time after them) in English
/// order: month, day, year, with slashes.
fn english_order(section: &Section) -> Option<Section> {
    let fields: Vec<&Part> = section
        .parts
        .iter()
        .filter(|part| !matches!(part, Part::Text(_)))
        .collect();
    let day = fields.iter().find_map(|part| match part {
        Part::Day(long) => Some(*long),
        _ => None,
    })?;
    let (month, textual) = fields.iter().find_map(|part| match part {
        Part::Month { long, textual } => Some((*long, *textual)),
        _ => None,
    })?;
    let weekday = fields.iter().find_map(|part| match part {
        Part::DayOfWeek(long) => Some(*long),
        _ => None,
    });
    let year = fields.iter().find_map(|part| match part {
        Part::Year(long) => Some(*long),
        _ => None,
    })?;
    // The date's three fields first, then whatever follows the year.
    let date_end = section
        .parts
        .iter()
        .rposition(|part| matches!(part, Part::Day(_) | Part::Month { .. } | Part::Year(_)))?;
    let leading = usize::from(weekday.is_some());
    if fields.len() > 3 + leading
        && fields[leading..3 + leading]
            .iter()
            .any(|part| !matches!(part, Part::Day(_) | Part::Month { .. } | Part::Year(_)))
    {
        return None;
    }
    let mut parts = Vec::new();
    if textual {
        // English's long date: `Sat, October 29, 2022`.
        if let Some(long) = weekday {
            parts.push(Part::DayOfWeek(long));
            parts.push(Part::Text(", ".to_string()));
        }
        parts.extend([
            Part::Month {
                long: month,
                textual: true,
            },
            Part::Text(" ".to_string()),
            Part::Day(day),
            Part::Text(", ".to_string()),
            Part::Year(year),
        ]);
    } else {
        if weekday.is_some() {
            return None;
        }
        parts.extend([
            Part::Month {
                long: month,
                textual: false,
            },
            Part::Text("/".to_string()),
            Part::Day(day),
            Part::Text("/".to_string()),
            Part::Year(year),
        ]);
    }
    parts.extend(section.parts[date_end + 1..].iter().cloned());
    Some(Section {
        parts,
        ..section.clone()
    })
}

fn render_date(section: &Section, value: f64) -> String {
    // Cut to the smallest unit shown (18:04:59.99 is 6:04 PM), with half a
    // millisecond's grace for a value stored a hair under its unit, as
    // LibreOffice shows a time.
    let decimals = section
        .parts
        .iter()
        .find_map(|part| match part {
            Part::Seconds { decimals, .. } => Some((*decimals).min(6)),
            _ => None,
        })
        .unwrap_or(0);
    let per_second = 10i64.pow(u32::from(decimals));
    let ticks = (value * 86_400.0 * per_second as f64 + 0.0005 * per_second as f64).floor() as i64;
    let per_day = 86_400 * per_second;
    let day_number = ticks.div_euclid(per_day);
    let tick_of_day = ticks.rem_euclid(per_day);
    let whole_seconds = tick_of_day / per_second;
    let (hour, minute, second) = (
        (whole_seconds / 3600) as u32,
        ((whole_seconds / 60) % 60) as u32,
        (whole_seconds % 60) as u32,
    );
    let fraction_ticks = tick_of_day % per_second;
    let days = day_number as f64;
    let total = ticks as f64 / per_day as f64;
    let (year, month, day) = crate::io::xlsx::reader::civil_from_days(days as i64 - 25_569);
    let weekday = ((days as i64 - 1).rem_euclid(7)) as usize;
    let twelve = section.parts.contains(&Part::AmPm);
    let mut out = String::new();
    for part in &section.parts {
        match part {
            Part::Text(text) => out.push_str(text),
            Part::Day(long) => pad(&mut out, i64::from(day), *long),
            Part::Month { long, textual } => {
                let name = MONTHS[(month as usize).saturating_sub(1) % 12];
                match (textual, long) {
                    (true, true) => out.push_str(name),
                    (true, false) => out.push_str(&name[..3]),
                    (false, _) => pad(&mut out, i64::from(month), *long),
                }
            }
            Part::Year(long) => {
                if *long {
                    let _ = write!(out, "{year:04}");
                } else {
                    let _ = write!(out, "{:02}", year.rem_euclid(100));
                }
            }
            Part::DayOfWeek(long) => {
                let name = WEEKDAYS[weekday];
                out.push_str(if *long { name } else { &name[..3] });
            }
            Part::Hours(long) => {
                let hours = if section.elapsed {
                    (total * 24.0).floor() as i64
                } else if twelve {
                    i64::from(match hour % 12 {
                        0 => 12,
                        other => other,
                    })
                } else {
                    i64::from(hour)
                };
                pad(&mut out, hours, *long);
            }
            Part::Minutes(long) => {
                let elapsed_minutes = section.elapsed
                    && !section
                        .parts
                        .iter()
                        .any(|part| matches!(part, Part::Hours(_)));
                let minutes = if elapsed_minutes {
                    (total * 1440.0).floor() as i64
                } else {
                    i64::from(minute)
                };
                pad(&mut out, minutes, *long);
            }
            Part::Seconds { long, decimals } => {
                let elapsed_seconds = section.elapsed
                    && !section
                        .parts
                        .iter()
                        .any(|part| matches!(part, Part::Hours(_) | Part::Minutes(_)));
                let seconds = if elapsed_seconds {
                    (total * 86_400.0).floor() as i64
                } else {
                    i64::from(second)
                };
                pad(&mut out, seconds, *long);
                if *decimals > 0 {
                    let _ = write!(
                        out,
                        ".{:0width$}",
                        fraction_ticks,
                        width = usize::from(*decimals)
                    );
                }
            }
            Part::AmPm => out.push_str(if hour < 12 { "AM" } else { "PM" }),
            _ => {}
        }
    }
    out
}

fn pad(out: &mut String, value: i64, long: bool) {
    if long {
        let _ = write!(out, "{value:02}");
    } else {
        let _ = write!(out, "{value}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(code: &str) -> String {
        to_excel(&from_excel(code).expect(code))
    }

    #[test]
    fn excel_codes_survive_the_model() {
        for code in [
            "0.00",
            "#,##0",
            "#,##0.00",
            "0%",
            "0.00%",
            "0.00E+00",
            "yyyy-mm-dd",
            "dd/mm/yyyy hh:mm:ss",
            "[hh]:mm:ss",
            "h:mm AM/PM",
            "dddd, mmmm d, yyyy",
            "# ?/?",
            "# ??/16",
            "[$€]#,##0.00",
            "#,##0.00;[Red]-#,##0.00",
            "0.0#",
        ] {
            assert_eq!(round_trip(code), code, "{code}");
        }
    }

    #[test]
    fn minutes_and_months_are_told_apart() {
        let style = from_excel("m/d/yy h:mm").expect("date");
        let parts = &style.sections[0].parts;
        assert!(parts.contains(&Part::Month {
            long: false,
            textual: false
        }));
        assert!(parts.contains(&Part::Minutes(true)));
        assert_eq!(style.kind(), Kind::Date);
        assert_eq!(from_excel("[h]:mm").expect("time").kind(), Kind::Time);
    }

    #[test]
    fn general_and_text_need_no_style() {
        assert!(from_excel("General").is_none());
        assert_eq!(from_excel("@").expect("text").kind(), Kind::Text);
    }

    #[test]
    fn values_render_as_the_format_shows_them() {
        let render = |code: &str, value: f64| from_excel(code).expect(code).render(value);
        assert_eq!(render("#,##0.00", 1234.5), "1,234.50");
        assert_eq!(render("0.00", -2.675), "-2.68");
        assert_eq!(render("0%", 0.256), "26%");
        assert_eq!(render("0.00E+00", 12345.0), "1.23E+04");
        assert_eq!(render("#,##0.00;[Red]-#,##0.00", -5.0), "-5.00");
        assert_eq!(render("#,##0.00;(#,##0.00)", -5.0), "(5.00)");
        assert_eq!(render("[$€]#,##0.00", 3.0), "€3.00");
        assert_eq!(render("yyyy-mm-dd", 45_296.0), "2024-01-05");
        assert_eq!(
            render("dddd, mmmm d, yyyy", 45_296.0),
            "Friday, January 5, 2024"
        );
        assert_eq!(render("h:mm AM/PM", 0.75), "6:00 PM");
        assert_eq!(render("[hh]:mm:ss", 1.5), "36:00:00");
        assert_eq!(render("# ?/?", 1.5), "1 1/2");
        assert_eq!(render("0.0#", 2.0), "2.0");
        assert_eq!(render("000-0000", 5551234.0), "555-1234");
    }

    #[test]
    fn ods_styles_round_trip_through_xml() {
        for code in [
            "#,##0.00;[Red]-#,##0.00",
            "yyyy-mm-dd\"T\"hh:mm:ss",
            "[hh]:mm:ss",
            "0.00%",
        ] {
            let style = from_excel(code).expect(code);
            let mut xml = String::from("<office:styles>");
            to_ods(&mut xml, "N1", &style);
            xml.push_str("</office:styles>");
            let mut styles = OdsStyles::default();
            let mut reader = XmlReader::new(&xml);
            while let Some(event) = reader.next() {
                if let XmlEvent::Start {
                    name,
                    attributes,
                    self_closing,
                } = event
                {
                    styles.read_style(&mut reader, name, &attributes, self_closing);
                }
            }
            let back = styles.resolve("N1").expect("resolved");
            assert_eq!(to_excel(&back), code, "{xml}");
        }
    }
}
