//! iWork cell formats (`TSK.FormatStructArchive`): how Numbers (and a
//! Pages table) shows a cell's value. A table's cells point into its
//! format table by kind (number, currency, date, duration, text,
//! boolean); a custom format is a named entry of the document's custom
//! format list, with conditions that pick another format by value.
//!
//! The rules follow numbers-parser's reading of the format (MIT,
//! <https://github.com/masaccio/numbers-parser>), checked against Numbers'
//! own CSV export (`scripts/numbers-check`).

/// One format, as the archive states it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Format {
    pub kind: u32,
    pub decimal_places: u32,
    pub currency_code: String,
    pub negative_style: u32,
    pub show_thousands_separator: bool,
    pub use_accounting_style: bool,
    pub duration_style: u32,
    pub base: u32,
    pub base_places: u32,
    pub base_use_minus_sign: bool,
    pub fraction_accuracy: u32,
    pub date_time_format: String,
    pub duration_unit_largest: u32,
    pub duration_unit_smallest: u32,
    pub use_automatic_duration_units: bool,
    /// The custom format this one stands for, by its list uuid.
    pub custom_uid: Option<(u64, u64)>,
    pub custom_format_string: String,
    pub scale_factor: f64,
    pub requires_fraction_replacement: bool,
    pub num_nonspace_integer_digits: u32,
    pub num_nonspace_decimal_digits: u32,
    /// Custom number formats: whether the format pads (Numbers' "complex"
    /// formats), has integer digits at all, the integer slots it keeps,
    /// and the decimal slots it pads.
    pub is_complex: bool,
    pub contains_integer_token: bool,
    pub min_integer_width: u32,
    pub decimal_width: u32,
}

/// A custom format: its default and the conditions that replace it.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CustomFormat {
    pub format: Format,
    pub conditions: Vec<Condition>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Condition {
    pub kind: u32,
    pub value: f64,
    pub format: Format,
}

pub const BOOLEAN: u32 = 1;
pub const DECIMAL: u32 = 256;
pub const CURRENCY: u32 = 257;
pub const PERCENT: u32 = 258;
pub const SCIENTIFIC: u32 = 259;
pub const FRACTION: u32 = 262;
pub const CHECKBOX: u32 = 263;
pub const RATING: u32 = 267;
pub const BASE: u32 = 269;
pub const CUSTOM_TEXT: u32 = 271;
pub const CUSTOM_DATE: u32 = 272;

/// `decimal_places` at or above this means "as many as the value needs".
const DECIMAL_PLACES_AUTO: u32 = 253;
const SIGNIFICANT_DIGITS: i32 = 15;
/// Where a custom text format puts the cell's text.
const TEXT_PLACEHOLDER: char = '\u{e421}';

const WEEK: u32 = 1;
const DAY: u32 = 2;
const HOUR: u32 = 4;
const MINUTE: u32 = 8;
const SECOND: u32 = 16;
const MILLISECOND: u32 = 32;

/// Currency symbols that are not their code followed by a space.
const CURRENCY_SYMBOLS: [(&str, &str); 21] = [
    ("AUD", "A$"),
    ("BRL", "R$"),
    ("CAD", "CA$"),
    ("CNY", "CN¥"),
    ("EUR", "€"),
    ("GBP", "£"),
    ("HKD", "HK$"),
    ("ILS", "₪"),
    ("INR", "₹"),
    ("JPY", "JP¥"),
    ("KRW", "₩"),
    ("MXN", "MX$"),
    ("NZD", "NZ$"),
    ("THB", "฿"),
    ("TWD", "NT$"),
    ("USD", "$"),
    ("VND", "₫"),
    ("XAF", "FCFA"),
    ("XCD", "EC$"),
    ("XOF", "CFA"),
    ("XPF", "CFPF"),
];

/// A number as plain decimal text (`-1234.5`), from the cell's exact
/// decimal when it has one, else from the double; at most fifteen
/// significant digits, as Numbers shows numbers, trailing zeros dropped.
pub fn plain(value: f64, exact: Option<&str>) -> String {
    let text = match exact {
        Some(text) if !text.contains(['E', 'e']) => text.to_string(),
        _ => expand(value),
    };
    trim_fraction(&round_significant(&text, SIGNIFICANT_DIGITS))
}

/// A double's shortest decimal that reads back to it, as plain text, with
/// an exact tie rounded to even (`123456789012345.125` is `...345.12`), as
/// Numbers writes it.
fn shortest(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let short = format!("{value:e}");
    let (mantissa, _) = short.split_once('e').unwrap_or((&short, "0"));
    let count = mantissa.chars().filter(char::is_ascii_digit).count().max(1);
    let text = format!("{:.*e}", count - 1, value.abs());
    let (mantissa, power) = text.split_once('e').unwrap_or((&text, "0"));
    let power: i32 = power.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if value < 0.0 { "-" } else { "" };
    sign.to_string() + &trim_fraction(&place_point(&digits, power + 1))
}

/// A double as plain decimal text with fifteen significant digits.
fn expand(value: f64) -> String {
    if value == 0.0 || !value.is_finite() {
        return "0".to_string();
    }
    let text = format!("{:.*e}", (SIGNIFICANT_DIGITS - 1) as usize, value.abs());
    let (mantissa, power) = text.split_once('e').unwrap_or((&text, "0"));
    let power: i32 = power.parse().unwrap_or(0);
    let digits: String = mantissa.chars().filter(char::is_ascii_digit).collect();
    let sign = if value < 0.0 { "-" } else { "" };
    sign.to_string() + &place_point(&digits, power + 1)
}

/// `digits` with the decimal point after `integers` of them (negative or
/// past the end pads with zeros).
fn place_point(digits: &str, integers: i32) -> String {
    if integers <= 0 {
        format!("0.{}{digits}", "0".repeat((-integers) as usize))
    } else if integers as usize >= digits.len() {
        format!("{digits}{}", "0".repeat(integers as usize - digits.len()))
    } else {
        let (whole, fraction) = digits.split_at(integers as usize);
        format!("{whole}.{fraction}")
    }
}

/// Splits `-12.50` into (negative, "12", "50").
fn parts(text: &str) -> (bool, &str, &str) {
    let (negative, text) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (whole, fraction) = text.split_once('.').unwrap_or((text, ""));
    (negative, whole, fraction)
}

/// Rounds plain decimal text half away from zero to `places` decimals,
/// written with exactly that many.
fn round_places(text: &str, places: usize) -> String {
    let (negative, whole, fraction) = parts(text);
    let mut digits: Vec<u8> = whole
        .bytes()
        .chain(fraction.bytes())
        .map(|b| b - b'0')
        .collect();
    let keep = whole.len() + places;
    digits.resize(digits.len().max(keep), 0);
    let up = digits.get(keep).is_some_and(|digit| *digit >= 5);
    digits.truncate(keep);
    if up {
        let mut index = digits.len();
        loop {
            if index == 0 {
                digits.insert(0, 1);
                break;
            }
            index -= 1;
            if digits[index] == 9 {
                digits[index] = 0;
            } else {
                digits[index] += 1;
                break;
            }
        }
    }
    let integers = digits.len() - places;
    let mut out: String = digits
        .iter()
        .map(|digit| char::from(b'0' + digit))
        .collect();
    if places > 0 {
        out.insert(integers, '.');
    }
    let out = out.trim_start_matches('0');
    let out = if out.is_empty() || out.starts_with('.') {
        format!("0{out}")
    } else {
        out.to_string()
    };
    let zero = out.bytes().all(|byte| matches!(byte, b'0' | b'.'));
    if negative && !zero {
        format!("-{out}")
    } else {
        out
    }
}

/// Rounds plain decimal text to `count` significant digits.
fn round_significant(text: &str, count: i32) -> String {
    let (_, whole, fraction) = parts(text);
    let whole = whole.trim_start_matches('0');
    let leading = if whole.is_empty() {
        -(fraction.bytes().take_while(|byte| *byte == b'0').count() as i32)
    } else {
        whole.len() as i32
    };
    let places = count - leading;
    if places >= fraction.len() as i32 {
        return text.to_string();
    }
    if places >= 0 {
        return round_places(text, places as usize);
    }
    // Rounding inside the integer: round at the point, then zero the tail.
    let rounded = round_places(&place_digits(text, places), 0);
    let (negative, digits, _) = parts(&rounded);
    let out = format!("{digits}{}", "0".repeat((-places) as usize));
    if negative { format!("-{out}") } else { out }
}

/// Moves the point of plain decimal text `shift` places left (negative) or
/// right.
fn place_digits(text: &str, shift: i32) -> String {
    let (negative, whole, fraction) = parts(text);
    let digits = format!("{whole}{fraction}");
    let out = trim_fraction(&place_point(&digits, whole.len() as i32 + shift));
    let out = out.trim_start_matches('0');
    let out = if out.is_empty() || out.starts_with('.') {
        format!("0{out}")
    } else {
        out.to_string()
    };
    if negative { format!("-{out}") } else { out }
}

/// Drops a fraction's trailing zeros, and the point with them.
fn trim_fraction(text: &str) -> String {
    if !text.contains('.') {
        return text.to_string();
    }
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// `1234567` -> `1,234,567`.
fn group(integer: &str) -> String {
    let mut out = String::with_capacity(integer.len() + integer.len() / 3);
    for (index, digit) in integer.chars().enumerate() {
        if index > 0 && (integer.len() - index) % 3 == 0 {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// The unsigned amount of a decimal, currency, or percentage: its places
/// (or as many as it needs) and its thousands separators.
fn amount(text: &str, format: &Format) -> String {
    let text = text.trim_start_matches('-');
    let text = if format.decimal_places >= DECIMAL_PLACES_AUTO {
        trim_fraction(&round_significant(text, SIGNIFICANT_DIGITS))
    } else {
        round_places(text, format.decimal_places as usize)
    };
    if !format.show_thousands_separator {
        return text;
    }
    match text.split_once('.') {
        Some((whole, fraction)) => format!("{}.{fraction}", group(whole)),
        None => group(&text),
    }
}

/// A negative value's form: a minus, none (shown in red), or parentheses.
fn signed(body: String, negative: bool, format: &Format) -> String {
    match (negative, format.negative_style) {
        (false, _) | (true, 1) => body,
        (true, 0) => format!("-{body}"),
        (true, _) => format!("({body})"),
    }
}

fn is_negative(text: &str) -> bool {
    text.starts_with('-') && text.bytes().any(|byte| matches!(byte, b'1'..=b'9'))
}

fn currency_symbol(code: &str) -> String {
    CURRENCY_SYMBOLS
        .iter()
        .find(|(known, _)| *known == code)
        .map_or_else(
            || format!("{code}\u{a0}"),
            |(_, symbol)| (*symbol).to_string(),
        )
}

fn scientific(text: &str, places: u32) -> String {
    let value: f64 = text.parse().unwrap_or(0.0);
    let auto = places >= DECIMAL_PLACES_AUTO;
    let digits = if auto {
        (SIGNIFICANT_DIGITS - 1) as usize
    } else {
        places.min(30) as usize
    };
    let written = format!("{value:.digits$e}");
    let (mantissa, power) = written.split_once('e').unwrap_or((&written, "0"));
    let mantissa = if auto {
        trim_fraction(mantissa)
    } else {
        mantissa.to_string()
    };
    let power: i32 = power.parse().unwrap_or(0);
    let sign = if power < 0 { '-' } else { '+' };
    format!("{mantissa}E{sign}{:02}", power.unsigned_abs())
}

fn fraction_parts(whole: i64, numerator: i64, denominator: i64) -> String {
    if whole > 0 {
        if numerator == 0 {
            return whole.to_string();
        }
        return format!("{whole} {numerator}/{denominator}");
    }
    if numerator == 0 {
        return "0".to_string();
    }
    if numerator == denominator {
        return "1".to_string();
    }
    format!("{numerator}/{denominator}")
}

/// The closest fraction with a denominator at most `limit` (Stern-Brocot).
fn closest_fraction(value: f64, limit: i64) -> (i64, i64) {
    let (mut low_n, mut low_d, mut high_n, mut high_d) = (0i64, 1i64, 1i64, 0i64);
    let mut best = (value.round() as i64, 1i64);
    let mut best_error = (value - value.round()).abs();
    for _ in 0..10_000 {
        let n = low_n + high_n;
        let d = low_d + high_d;
        if d > limit {
            break;
        }
        let error = (value - n as f64 / d as f64).abs();
        if error < best_error {
            best = (n, d);
            best_error = error;
        }
        if (n as f64) < value * d as f64 {
            low_n = n;
            low_d = d;
        } else if (n as f64) > value * d as f64 {
            high_n = n;
            high_d = d;
        } else {
            break;
        }
    }
    best
}

fn fraction(value: f64, format: &Format) -> String {
    let accuracy = format.fraction_accuracy;
    let whole = value.trunc() as i64;
    if accuracy & 0xFF00_0000 != 0 {
        let digits = 0x1_0000_0000u64 - u64::from(accuracy);
        let limit = 10i64.pow(digits.min(9) as u32) - 1;
        let (numerator, denominator) = closest_fraction(value, limit);
        return fraction_parts(whole, numerator - whole * denominator, denominator);
    }
    let denominator = i64::from(accuracy.max(1));
    let numerator = (denominator as f64 * (value - whole as f64)).round() as i64;
    fraction_parts(whole, numerator, denominator)
}

fn base(value: f64, format: &Format) -> String {
    let places = format.base_places as usize;
    let radix = u64::from(format.base.clamp(2, 36));
    let value = value.round() as i64;
    if value == 0 {
        return format!("{:0>places$}", "0");
    }
    let digits = |mut number: u64| {
        let mut out = Vec::new();
        while number > 0 {
            let digit = (number % radix) as u32;
            out.push(
                char::from_digit(digit, radix as u32)
                    .unwrap_or('0')
                    .to_ascii_uppercase(),
            );
            number /= radix;
        }
        out.iter().rev().collect::<String>()
    };
    if value < 0 && !format.base_use_minus_sign && matches!(radix, 2 | 8 | 16) {
        // Two's complement in at least 32 bits.
        let bits = (64 - value.unsigned_abs().leading_zeros() + 1).max(32);
        let mask = if bits >= 64 {
            u64::MAX
        } else {
            (1u64 << bits) - 1
        };
        return digits((value as u64) & mask);
    }
    let text = format!("{:0>places$}", digits(value.unsigned_abs()));
    if value < 0 { format!("-{text}") } else { text }
}

/// A number under its format: the cell's number, currency, or boolean
/// format, or the custom format it stands for. `exact` is the cell's
/// decimal as stored, when it has one.
pub fn number(
    value: f64,
    exact: Option<&str>,
    format: &Format,
    custom: Option<&CustomFormat>,
) -> String {
    // Numbers formats the value as a double: its shortest decimal, every
    // digit of it under explicit places, fifteen significant without.
    let _ = exact;
    let text = shortest(value);
    if let Some(custom) = custom {
        if custom.format.requires_fraction_replacement {
            return fraction(value, &custom.format);
        }
        return custom_number(&text, custom);
    }
    let negative = is_negative(&text);
    match format.kind {
        DECIMAL => signed(amount(&text, format), negative, format),
        CURRENCY => {
            let symbol = currency_symbol(&format.currency_code);
            let amount = amount(&text, format);
            if format.use_accounting_style {
                return if negative {
                    format!("{symbol}\t({amount})")
                } else {
                    format!("{symbol}\t{amount}")
                };
            }
            signed(format!("{symbol}{amount}"), negative, format)
        }
        PERCENT => signed(
            amount(&place_digits(&text, 2), format) + "%",
            negative,
            format,
        ),
        SCIENTIFIC => scientific(&text, format.decimal_places),
        FRACTION => fraction(value, format),
        BASE => base(value, format),
        // Numbers exports a checkbox as its truth and a rating as its
        // number of stars.
        BOOLEAN | CHECKBOX => if value != 0.0 { "TRUE" } else { "FALSE" }.to_string(),
        RATING => plain(value, exact),
        _ => plain(value, exact),
    }
}

/// A custom text format: the cell's text set into the format's string.
pub fn custom_text(text: &str, custom: &CustomFormat) -> String {
    custom
        .format
        .custom_format_string
        .replace(TEXT_PLACEHOLDER, text)
}

/// Removes the single quotes a format uses to mark literal text (`''` is
/// a quote).
fn expand_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\'' {
            if chars.peek() == Some(&'\'') {
                out.push('\'');
                chars.next();
            }
            continue;
        }
        out.push(character);
    }
    out
}

/// A custom number format (`#,##0.00 'units'`), with its conditions.
/// Integer `0`s pad with zeros, or with spaces when the format keeps none
/// of them as zeros; integer `#`s show at least one digit. Decimal `0`s
/// always show, `#`s only when not trailing zeros, and a format with no
/// integer digits keeps one decimal.
fn custom_number(text: &str, custom: &CustomFormat) -> String {
    let mut format = &custom.format;
    let negative = is_negative(text);
    let mut sign = if negative { "-" } else { "" };
    let value: f64 = text.parse().unwrap_or(0.0);
    let rounded: f64 = plain(value, Some(text)).parse().unwrap_or(value);
    for condition in &custom.conditions {
        let matches = match condition.kind {
            0 => rounded == condition.value,
            1 => rounded < condition.value,
            2 => rounded <= condition.value,
            3 => rounded > condition.value,
            4 => rounded >= condition.value,
            _ => false,
        };
        if matches {
            format = &condition.format;
            // Numbers drops the sign when a condition applies.
            sign = "";
            break;
        }
    }
    let mut pattern = format.custom_format_string.clone();
    let mut magnitude = text.trim_start_matches('-').to_string();
    let scale = custom.format.scale_factor;
    if scale != 0.0 && scale != 1.0 {
        magnitude = expand(value.abs() * scale);
    } else if pattern.contains('%') {
        magnitude = place_digits(&magnitude, 2);
    }
    if !format.currency_code.is_empty() {
        pattern = pattern.replace('\u{a4}', &currency_symbol(&format.currency_code));
    }
    let Some((start, end)) = number_spec(&pattern) else {
        return expand_quotes(&pattern);
    };
    let after = &pattern[end..];
    let exponent = after.starts_with("E+") && after[2..].starts_with(|c: char| c.is_ascii_digit());
    let spec_end = if exponent {
        end + 2 + after[2..].chars().take_while(char::is_ascii_digit).count()
    } else {
        end
    };
    let (integer_spec, decimal_spec) = pattern[start..end]
        .split_once('.')
        .unwrap_or((&pattern[start..end], ""));
    let shown = if exponent {
        scientific(&magnitude, decimal_spec.len() as u32)
    } else {
        custom_digits(&magnitude, integer_spec, decimal_spec, format)
    };
    sign.to_string()
        + &expand_quotes(&format!(
            "{}{shown}{}",
            &pattern[..start],
            &pattern[spec_end..]
        ))
}

/// Where a custom format's number goes: the first run of `#0.,` outside
/// quoted text.
fn number_spec(pattern: &str) -> Option<(usize, usize)> {
    let is_spec = |c: char| matches!(c, '#' | '0' | '.' | ',');
    let mut quoted = false;
    let mut start = None;
    for (index, character) in pattern.char_indices() {
        if character == '\'' {
            if start.is_some() {
                return start.map(|start| (start, index));
            }
            quoted = !quoted;
            continue;
        }
        match start {
            None if !quoted && is_spec(character) => start = Some(index),
            Some(start) if !is_spec(character) => return Some((start, index)),
            _ => {}
        }
    }
    start.map(|start| (start, pattern.len()))
}

/// A custom format's digits. A simple format shows the integer (at least
/// `0`) and its decimals, `#` decimals without trailing zeros. A complex
/// format keeps `min_integer_width` integer slots, the rightmost
/// `num_nonspace_integer_digits` as zeros and the rest as spaces (unused
/// `#` decimals turn space slots into zeros), and pads `0` decimals with
/// zeros, or with spaces when it has integer digits and no zero decimals.
fn custom_digits(
    magnitude: &str,
    integer_spec: &str,
    decimal_spec: &str,
    format: &Format,
) -> String {
    let decimal_zeros = decimal_spec.chars().filter(|c| *c == '0').count();
    let decimal_hashes = decimal_spec.chars().filter(|c| *c == '#').count();
    let rounded = round_places(magnitude, decimal_zeros + decimal_hashes);
    let (_, whole, fraction) = parts(&rounded);
    let whole = whole.trim_start_matches('0');
    let thousands = format.show_thousands_separator;
    let complex = format.is_complex;
    let mut fraction = fraction.to_string();
    let keep_zeros =
        !complex || format.num_nonspace_decimal_digits > 0 || !format.contains_integer_token;
    let minimum = if !complex && !format.contains_integer_token && !decimal_spec.is_empty() {
        decimal_zeros.max(1)
    } else if keep_zeros {
        decimal_zeros
    } else {
        0
    };
    while fraction.len() > minimum && fraction.ends_with('0') {
        fraction.pop();
    }
    let used_hashes = fraction.len().saturating_sub(decimal_zeros);
    if complex && !keep_zeros && fraction.len() < decimal_zeros {
        fraction = format!("{fraction:<decimal_zeros$}");
    }
    let integer = if !complex {
        let digits = if whole.is_empty() { "0" } else { whole };
        if thousands {
            group(digits)
        } else {
            digits.to_string()
        }
    } else {
        // A fixed-width field: the integer slots (and their separators),
        // the point, and the decimals. Width the number leaves unused on
        // the right (optional decimals it does not need, and the point
        // when none show) becomes zeros on the left of the integer; the
        // rightmost `num_nonspace_integer_digits` slots are always zeros;
        // the rest are spaces.
        let width = (format.min_integer_width as usize)
            .max(integer_spec.chars().filter(|c| *c == '0').count());
        let mut freed = decimal_hashes - used_hashes.min(decimal_hashes);
        if !decimal_spec.is_empty() && fraction.is_empty() {
            freed += 1;
        }
        let digits: Vec<char> = whole.chars().collect();
        let zeros = format.num_nonspace_integer_digits as usize;
        // Built from the right: slot k is the k-th digit from the right.
        let mut out: Vec<char> = Vec::new();
        let slots = width.max(digits.len());
        for k in 0..slots {
            if k > 0 && k % 3 == 0 && thousands {
                let comma = if k < digits.len() || k < zeros {
                    ','
                } else if freed > 0 {
                    freed -= 1;
                    ','
                } else {
                    ' '
                };
                out.push(comma);
            }
            let slot = if k < digits.len() {
                digits[digits.len() - 1 - k]
            } else if k < zeros {
                '0'
            } else if freed > 0 {
                freed -= 1;
                '0'
            } else {
                ' '
            };
            out.push(slot);
        }
        out.iter().rev().collect()
    };
    if fraction.is_empty() {
        integer
    } else {
        format!("{integer}.{fraction}")
    }
}

/// How a formatted number reaches Excel.
#[derive(Debug, Clone, PartialEq)]
pub enum Excel {
    /// A number in Excel's default format.
    General,
    /// A number shown with this Excel number format.
    Code(String),
    /// A boolean.
    Boolean,
    /// No Excel format shows it the same: the text as Numbers shows it.
    Text,
}

/// In an Excel code from `excel`, where the value's own decimals go.
pub const AUTO_DECIMALS: char = '\u{1}';

/// An Excel code with its `AUTO_DECIMALS` replaced by the decimals
/// `value` has (none, or `.` and a zero for each).
pub fn excel_places(code: &str, value: f64) -> String {
    if !code.contains(AUTO_DECIMALS) {
        return code.to_string();
    }
    let mut text = plain(value, None);
    if code.contains('%') {
        // A percentage shows the value a hundred times over.
        text = place_digits(&text, 2);
    }
    let places = text
        .split_once('.')
        .map_or(0, |(_, fraction)| fraction.len());
    let decimals = if places > 0 {
        format!(".{}", "0".repeat(places))
    } else {
        String::new()
    };
    code.replace(AUTO_DECIMALS, &decimals)
}

/// The Excel number format that shows a number as `format` does.
pub fn excel(format: &Format, custom: Option<&CustomFormat>) -> Excel {
    if let Some(custom) = custom {
        return excel_custom(custom);
    }
    let digits = |format: &Format| {
        let mut code = String::from(if format.show_thousands_separator {
            "#,##0"
        } else {
            "0"
        });
        if format.decimal_places >= DECIMAL_PLACES_AUTO {
            // As many decimals as the value has: `excel_places` fills them
            // in per cell (Excel's `#` decimals leave a stray point).
            code.push(AUTO_DECIMALS);
        } else if format.decimal_places > 0 {
            code.push('.');
            code.push_str(&"0".repeat(format.decimal_places.min(30) as usize));
        }
        code
    };
    let negatives = |positive: String, format: &Format| match format.negative_style {
        1 => format!("{positive};[Red]{positive}"),
        2 => format!("{positive};({positive})"),
        3 => format!("{positive};[Red]({positive})"),
        _ => positive,
    };
    match format.kind {
        DECIMAL
            if format.decimal_places >= DECIMAL_PLACES_AUTO
                && !format.show_thousands_separator
                && format.negative_style == 0 =>
        {
            Excel::General
        }
        DECIMAL => Excel::Code(negatives(digits(format), format)),
        CURRENCY => {
            let symbol = currency_symbol(&format.currency_code);
            let quoted = format!("\"{}\"", symbol.replace('"', ""));
            Excel::Code(negatives(format!("{quoted}{}", digits(format)), format))
        }
        PERCENT => Excel::Code(negatives(format!("{}%", digits(format)), format)),
        SCIENTIFIC => {
            let fraction = if format.decimal_places >= DECIMAL_PLACES_AUTO {
                // As many digits as the number has, as Numbers shows it.
                format!(".{}", "#".repeat(14))
            } else if format.decimal_places > 0 {
                format!(".{}", "0".repeat(format.decimal_places.min(30) as usize))
            } else {
                String::new()
            };
            Excel::Code(format!("0{fraction}E+00"))
        }
        FRACTION => Excel::Code(match format.fraction_accuracy {
            0xFFFF_FFFF => "# ?/?".to_string(),
            0xFFFF_FFFE => "# ??/??".to_string(),
            0xFFFF_FFFD => "# ???/???".to_string(),
            denominator => format!("# ?/{}", denominator.max(1)),
        }),
        BOOLEAN | CHECKBOX => Excel::Boolean,
        RATING => Excel::General,
        BASE => Excel::Text,
        _ => Excel::General,
    }
}

/// A custom number format in Excel's terms: digits, separators, percent,
/// exponent, and quoted text carry over; conditions, and the padding
/// Numbers' complex formats do, do not (shown as text).
fn excel_custom(custom: &CustomFormat) -> Excel {
    let format = &custom.format;
    if !custom.conditions.is_empty()
        || format.requires_fraction_replacement
        || format.kind == CUSTOM_TEXT
    {
        return Excel::Text;
    }
    let mut code = String::new();
    let mut quoted = String::new();
    let mut in_quote = false;
    let mut chars = format.custom_format_string.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\'' {
            if chars.peek() == Some(&'\'') {
                chars.next();
                quoted.push('\'');
                continue;
            }
            in_quote = !in_quote;
            continue;
        }
        if in_quote {
            quoted.push(character);
            continue;
        }
        if !quoted.is_empty() {
            code.push_str(&format!("\"{}\"", quoted.replace('"', "")));
            quoted.clear();
        }
        match character {
            '#' | '0' | '.' | ',' | '%' | 'E' | '+' | '-' => code.push(character),
            '\u{a4}' => code.push_str(&format!(
                "\"{}\"",
                currency_symbol(&format.currency_code).replace('"', "")
            )),
            ' ' => code.push(' '),
            other if other.is_ascii_digit() => code.push(other),
            other => code.push_str(&format!("\"{other}\"")),
        }
    }
    if !quoted.is_empty() {
        code.push_str(&format!("\"{}\"", quoted.replace('"', "")));
    }
    if !format.show_thousands_separator {
        code = code.replace(',', "");
    }
    if format.is_complex {
        // Numbers' padding in Excel's terms: a zero slot is `0`, a space
        // slot `?` (a digit or a space).
        let end = code
            .find(|c: char| !matches!(c, '#' | '0' | ',' | '.'))
            .unwrap_or(code.len());
        if let Some(start) = code[..end].find(['#', '0', '.']) {
            let spec = code[start..end].to_string();
            let (integer, decimals) = spec.split_once('.').unwrap_or((&spec, ""));
            let width = (format.min_integer_width as usize).max(integer.matches('0').count());
            let zeros = (format.num_nonspace_integer_digits as usize).min(width);
            let mut digits = "?".repeat(width - zeros) + &"0".repeat(zeros);
            if width == 0 {
                digits = "#".to_string();
            }
            if format.show_thousands_separator {
                let mut grouped = String::new();
                for (index, digit) in digits.chars().enumerate() {
                    if index > 0 && (digits.len() - index) % 3 == 0 {
                        grouped.push(',');
                    }
                    grouped.push(digit);
                }
                digits = if grouped.contains(',') {
                    grouped
                } else {
                    format!("#,##{grouped}")
                };
            }
            let keep_zeros =
                format.num_nonspace_decimal_digits > 0 || !format.contains_integer_token;
            let decimals: String = decimals
                .chars()
                .map(|c| if c == '0' && !keep_zeros { '?' } else { c })
                .collect();
            let replacement = if spec.contains('.') {
                format!("{digits}.{decimals}")
            } else {
                digits
            };
            code.replace_range(start..end, &replacement);
        }
        return Excel::Code(code);
    }
    if !format.is_complex {
        // Numbers groups thousands when the format says to, separators
        // written or not.
        let end = code
            .find(|c: char| !matches!(c, '#' | '0' | ','))
            .unwrap_or(code.len());
        let start = code[..end].find(['#', '0']).unwrap_or(end);
        if format.show_thousands_separator && start < end && !code[start..end].contains(',') {
            code.insert_str(start, "#,##");
        }
        // An integer-only format still shows the integer's zero.
        if !code.contains('.') && !code.contains('0') {
            if let Some(last) = code.rfind('#') {
                code.replace_range(last..=last, "0");
            }
        }
        // Numbers shows the integer's zero, and a decimal when the format
        // has no integer digits.
        if let Some(point) = code.find('.') {
            let integer = code[..point].to_string();
            match integer.rfind('#') {
                Some(last) if !integer.contains('0') => code.replace_range(last..=last, "0"),
                None if !integer.contains('0') => {
                    code.insert(point, '0');
                    if code[point + 2..].starts_with('#') {
                        code.replace_range(point + 2..point + 3, "0");
                    }
                }
                _ => {}
            }
        }
    }
    if format.scale_factor != 0.0 && format.scale_factor != 1.0 {
        // Excel scales by thousands with trailing commas only.
        return Excel::Text;
    }
    Excel::Code(code)
}

/// An ICU date pattern as an Excel date format, when Excel can show it.
pub fn excel_date(pattern: &str) -> Option<String> {
    let chars: Vec<char> = pattern.chars().collect();
    let has_period = chars.contains(&'a');
    let mut out = String::new();
    let mut index = 0;
    let mut quoted = false;
    while index < chars.len() {
        let character = chars[index];
        if character == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                out.push_str("\"'\"");
                index += 2;
            } else {
                quoted = !quoted;
                index += 1;
            }
            continue;
        }
        if quoted || !character.is_ascii_alphabetic() {
            match character {
                '"' => {}
                c if quoted || !matches!(c, ' ' | '/' | '-' | ':' | ',' | '.') => {
                    out.push('"');
                    out.push(c);
                    out.push('"');
                }
                c => out.push(c),
            }
            index += 1;
            continue;
        }
        let run = chars[index..]
            .iter()
            .take_while(|&&c| c == character)
            .count();
        let token = match (character, run) {
            ('y', 2) => "yy",
            ('y', _) => "yyyy",
            ('M' | 'L', 1) => "m",
            ('M' | 'L', 2) => "mm",
            ('M' | 'L', 3) => "mmm",
            ('M' | 'L', _) => "mmmm",
            ('d', 1) => "d",
            ('d', _) => "dd",
            ('E', 1..=3) => "ddd",
            ('E', _) => "dddd",
            ('H', 1) | ('h', 1) => "h",
            ('H', _) | ('h', _) => "hh",
            ('m', 1) => "m",
            ('m', _) => "mm",
            ('s', 1) => "s",
            ('s', _) => "ss",
            ('a', _) => "AM/PM",
            ('S', count) => {
                // Fractions of a second follow the seconds in Excel.
                if !out.ends_with('s') {
                    return None;
                }
                out.push('.');
                out.push_str(&"0".repeat(count.min(3)));
                index += run;
                continue;
            }
            _ => return None,
        };
        if character == 'H' && has_period {
            return None;
        }
        out.push_str(token);
        index += run;
    }
    Some(out)
}

/// A compact duration format as an Excel elapsed-time format, when its
/// units stop at hours.
pub fn excel_duration(format: &Format) -> Option<String> {
    let (largest, smallest) = (format.duration_unit_largest, format.duration_unit_smallest);
    if format.duration_style != 0
        || format.use_automatic_duration_units
        || !(HOUR..=SECOND).contains(&largest)
    {
        return None;
    }
    // Excel's elapsed time: the largest unit in brackets, the rest after.
    // Minutes or seconds first pad to two digits when another unit
    // follows, as Numbers shows them; hours never pad.
    let alone = largest == smallest;
    let mut code = String::from(match (largest, alone) {
        (HOUR, _) => "[h]",
        (MINUTE, true) => "[m]",
        (MINUTE, false) => "[mm]",
        (_, true) => "[s]",
        (_, false) => "[ss]",
    });
    if largest < MINUTE && smallest >= MINUTE {
        code.push_str(":mm");
    }
    if largest < SECOND && smallest >= SECOND {
        code.push_str(":ss");
    }
    if smallest >= MILLISECOND {
        code.push_str(".000");
    }
    Some(code)
}

/// A duration in seconds under its format.
pub fn duration(seconds: f64, format: &Format) -> String {
    let style = format.duration_style;
    let (mut smallest, mut largest) = (format.duration_unit_smallest, format.duration_unit_largest);
    if format.use_automatic_duration_units {
        (smallest, largest) = automatic_units(seconds, format);
    }
    let in_range = |unit: u32| largest <= unit && smallest >= unit;
    let unit = |name: &str, value: i64, short: &str| -> String {
        match style {
            0 => String::new(),
            1 => short.to_string(),
            _ => format!(" {name}{}", if value == 1 { "" } else { "s" }),
        }
    };
    let mut rest = seconds;
    let mut parts: Vec<String> = Vec::new();
    if largest == WEEK {
        let count = (rest / 604_800.0).trunc() as i64;
        if smallest != WEEK {
            rest -= 604_800.0 * count as f64;
        }
        parts.push(format!("{count}{}", unit("week", count, "w")));
    }
    for (step, size, name, short) in [(DAY, 86_400.0, "day", "d"), (HOUR, 3_600.0, "hour", "h")] {
        if in_range(step) {
            let count = (rest / size).trunc() as i64;
            if smallest > step {
                rest -= size * count as f64;
            }
            parts.push(format!("{count}{}", unit(name, count, short)));
        }
    }
    for (step, size, name, short) in [(MINUTE, 60.0, "minute", "m"), (SECOND, 1.0, "second", "s")] {
        if in_range(step) {
            let count = (rest / size).trunc() as i64;
            if smallest > step {
                rest -= size * count as f64;
            }
            if style == 0 {
                let alone = largest == step && smallest == step;
                parts.push(if alone || count >= 10 {
                    count.to_string()
                } else {
                    format!("0{count}")
                });
            } else {
                parts.push(format!("{count}{}", unit(name, count, short)));
            }
        }
    }
    if smallest >= MILLISECOND {
        let count = (1000.0 * rest).round() as i64;
        if style == 0 {
            parts.push(format!("{count:0>3}"));
        } else {
            parts.push(format!("{count}{}", unit("millisecond", count, "ms")));
        }
    }
    let joined = parts.join(if style == 0 { ":" } else { " " });
    if style == 0 && smallest >= MILLISECOND {
        // `1:02:003` -> `1:02.003`.
        if let Some(index) = joined.rfind(':') {
            return format!("{}.{}", &joined[..index], &joined[index + 1..]);
        }
    }
    joined
}

fn automatic_units(seconds: f64, format: &Format) -> (u32, u32) {
    if seconds == 0.0 {
        // Numbers shows zero in weeks.
        return (WEEK, WEEK);
    }
    let largest = if seconds >= 604_800.0 {
        WEEK
    } else if seconds >= 86_400.0 {
        DAY
    } else if seconds >= 3_600.0 {
        HOUR
    } else if seconds >= 60.0 {
        MINUTE
    } else if seconds >= 1.0 {
        SECOND
    } else {
        MILLISECOND
    };
    let smallest = if seconds.fract() != 0.0 {
        MILLISECOND
    } else if seconds % 60.0 != 0.0 {
        SECOND
    } else if seconds % 3_600.0 != 0.0 {
        MINUTE
    } else if seconds % 86_400.0 != 0.0 {
        HOUR
    } else if seconds % 604_800.0 != 0.0 {
        DAY
    } else {
        format.duration_unit_smallest
    };
    (smallest.max(largest), largest)
}

/// A calendar moment from seconds since 2001-01-01.
struct Moment {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
    /// 0 is Sunday.
    weekday: u32,
    day_of_year: u32,
    micros: u32,
}

impl Moment {
    fn from_seconds(seconds: f64) -> Moment {
        let total = seconds.floor() as i64;
        let micros = ((seconds - seconds.floor()) * 1_000_000.0).round() as u32;
        let days = total.div_euclid(86_400);
        let of_day = total.rem_euclid(86_400) as u32;
        // Days from 1970-01-01: 2001-01-01 is day 11,323.
        let unix_days = days + 11_323;
        let z = unix_days + 719_468;
        let era = z.div_euclid(146_097);
        let day_of_era = z - era * 146_097;
        let year_of_era =
            (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
        let doy = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
        let month_index = (5 * doy + 2) / 153;
        let day = (doy - (153 * month_index + 2) / 5 + 1) as u32;
        let month = if month_index < 10 {
            month_index + 3
        } else {
            month_index - 9
        } as u32;
        let year = year_of_era + era * 400 + i64::from(month <= 2);
        let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
        const STARTS: [u32; 12] = [0, 31, 59, 90, 120, 151, 181, 212, 243, 273, 304, 334];
        let day_of_year = STARTS[(month - 1) as usize] + day + u32::from(leap && month > 2);
        Moment {
            year,
            month,
            day,
            hour: of_day / 3600,
            minute: of_day % 3600 / 60,
            second: of_day % 60,
            // 1970-01-01 was a Thursday.
            weekday: (unix_days + 4).rem_euclid(7) as u32,
            day_of_year,
            micros,
        }
    }
}

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

/// A date under an ICU-style pattern (`EEE, d MMM yyyy HH:mm:ss`), the
/// form both built-in and custom date formats take.
pub fn date(seconds: f64, pattern: &str) -> String {
    let moment = Moment::from_seconds(seconds);
    let system_style = matches!(pattern, "h:mm a" | "M/d/yy h:mm a");
    let chars: Vec<char> = pattern.chars().collect();
    let mut out = String::new();
    let mut index = 0;
    let mut quoted = false;
    while index < chars.len() {
        let character = chars[index];
        if character == '\'' {
            if chars.get(index + 1) == Some(&'\'') {
                out.push('\'');
                index += 2;
            } else {
                quoted = !quoted;
                index += 1;
            }
            continue;
        }
        if quoted || !character.is_ascii_alphabetic() {
            // The system's short time style sets AM and PM off with a
            // narrow no-break space; other patterns keep their space.
            if character == ' ' && system_style && chars.get(index + 1) == Some(&'a') {
                out.push('\u{202f}');
            } else {
                out.push(character);
            }
            index += 1;
            continue;
        }
        let run = chars[index..]
            .iter()
            .take_while(|&&c| c == character)
            .count();
        // Fields longer than the pattern letter's longest form split.
        let (field, used) = date_field(&moment, character, run);
        out.push_str(&field);
        index += used;
    }
    out
}

/// One date field: its text and how many pattern letters it used.
fn date_field(moment: &Moment, letter: char, run: usize) -> (String, usize) {
    let twelve = |hour: u32| if hour % 12 == 0 { 12 } else { hour % 12 };
    match letter {
        'y' => match run {
            2 => (format!("{:02}", moment.year.rem_euclid(100)), 2),
            _ => (format!("{}", moment.year), run.min(4)),
        },
        'M' | 'L' => match run {
            1 => (moment.month.to_string(), 1),
            2 => (format!("{:02}", moment.month), 2),
            3 => (MONTHS[(moment.month - 1) as usize][..3].to_string(), 3),
            _ => (MONTHS[(moment.month - 1) as usize].to_string(), run.min(4)),
        },
        'd' => match run {
            1 => (moment.day.to_string(), 1),
            _ => (format!("{:02}", moment.day), 2),
        },
        'D' => (
            format!("{:0>width$}", moment.day_of_year, width = run.min(3)),
            run.min(3),
        ),
        'E' => match run {
            1..=3 => (WEEKDAYS[moment.weekday as usize][..3].to_string(), run),
            _ => (WEEKDAYS[moment.weekday as usize].to_string(), run.min(4)),
        },
        'H' => match run {
            1 => (moment.hour.to_string(), 1),
            _ => (format!("{:02}", moment.hour), 2),
        },
        'h' => match run {
            1 => (twelve(moment.hour).to_string(), 1),
            _ => (format!("{:02}", twelve(moment.hour)), 2),
        },
        'k' => {
            let hour = if moment.hour == 0 { 24 } else { moment.hour };
            match run {
                1 => (hour.to_string(), 1),
                _ => (format!("{hour:02}"), 2),
            }
        }
        'K' => match run {
            1 => ((moment.hour % 12).to_string(), 1),
            _ => (format!("{:02}", moment.hour % 12), 2),
        },
        'm' => match run {
            1 => (moment.minute.to_string(), 1),
            _ => (format!("{:02}", moment.minute), 2),
        },
        's' => match run {
            1 => (moment.second.to_string(), 1),
            _ => (format!("{:02}", moment.second), 2),
        },
        'S' => {
            let digits = format!("{:06}", moment.micros);
            let width = run.min(6);
            (digits[..width].to_string(), width)
        }
        'a' => (
            (if moment.hour < 12 { "AM" } else { "PM" }).to_string(),
            run,
        ),
        'G' => ("AD".to_string(), run),
        // Week of the year and of the month (weeks start on Sunday, the
        // first week holds day one), and which of its weekday the day is
        // in the month.
        'w' => {
            let first = (moment.weekday + 7 * 60 - (moment.day_of_year - 1)) % 7;
            let week = (moment.day_of_year + first - 1) / 7 + 1;
            (
                if run >= 2 {
                    format!("{week:02}")
                } else {
                    week.to_string()
                },
                run.min(2),
            )
        }
        'W' => {
            let first = (moment.weekday + 7 * 10 - (moment.day - 1)) % 7;
            (((moment.day + first - 1) / 7 + 1).to_string(), 1)
        }
        'F' => (((moment.day - 1) / 7 + 1).to_string(), 1),
        'Q' => (format!("Q{}", (moment.month - 1) / 3 + 1), run),
        _ => (String::new(), run),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal_format(places: u32, thousands: bool) -> Format {
        Format {
            kind: DECIMAL,
            decimal_places: places,
            show_thousands_separator: thousands,
            ..Format::default()
        }
    }

    #[test]
    fn numbers_take_their_places_and_separators() {
        assert_eq!(
            number(
                1877.0,
                None,
                &decimal_format(DECIMAL_PLACES_AUTO, true),
                None
            ),
            "1,877"
        );
        assert_eq!(
            number(1234.5, Some("1234.5"), &decimal_format(2, true), None),
            "1,234.50"
        );
        assert_eq!(number(-3.0, None, &decimal_format(0, false), None), "-3");
        let currency = Format {
            kind: CURRENCY,
            currency_code: "GBP".to_string(),
            show_thousands_separator: true,
            decimal_places: 0,
            ..Format::default()
        };
        assert_eq!(number(1877.0, None, &currency, None), "£1,877");
        let percent = Format {
            kind: PERCENT,
            decimal_places: 1,
            ..Format::default()
        };
        assert_eq!(number(0.256, Some("0.256"), &percent, None), "25.6%");
    }

    #[test]
    fn exact_decimals_round_half_away_and_show_fifteen_digits() {
        assert_eq!(
            number(0.125, Some("0.125"), &decimal_format(2, false), None),
            "0.13"
        );
        assert_eq!(
            number(-0.0005, Some("-0.0005"), &decimal_format(20, false), None),
            "-0.00050000000000000000"
        );
        assert_eq!(plain(29.999999999999996, None), "30");
        assert_eq!(
            plain(5.713383956445853e39, None),
            "5713383956445850000000000000000000000000"
        );
        let negative_currency = Format {
            kind: CURRENCY,
            currency_code: "GBP".to_string(),
            decimal_places: 2,
            ..Format::default()
        };
        assert_eq!(
            number(-302.7, Some("-302.7"), &negative_currency, None),
            "-£302.70"
        );
        let sci = Format {
            kind: SCIENTIFIC,
            decimal_places: DECIMAL_PLACES_AUTO,
            ..Format::default()
        };
        assert_eq!(number(1234.56, Some("1234.56"), &sci, None), "1.23456E+03");
        assert_eq!(
            shortest("123456789012345.125".parse().unwrap()),
            "123456789012345.12"
        );
        assert_eq!(shortest(0.0005), "0.0005");
    }

    #[test]
    fn custom_numbers_pad_and_trim_as_numbers_shows_them() {
        // (pattern, complex, zero integers, minimum integer slots, zero
        // decimals, thousands), as Numbers stores them.
        let custom = |pattern: &str,
                      complex: bool,
                      zeros: u32,
                      width: u32,
                      decimals: u32,
                      thousands: bool| {
            CustomFormat {
                format: Format {
                    custom_format_string: pattern.to_string(),
                    is_complex: complex,
                    contains_integer_token: !pattern.starts_with('.'),
                    num_nonspace_integer_digits: zeros,
                    min_integer_width: width,
                    num_nonspace_decimal_digits: decimals,
                    show_thousands_separator: thousands,
                    scale_factor: 1.0,
                    ..Format::default()
                },
                conditions: Vec::new(),
            }
        };
        let shown = |value: &str, format: &CustomFormat| {
            number(
                value.parse().unwrap(),
                Some(value),
                &Format::default(),
                Some(format),
            )
        };
        // Simple formats.
        assert_eq!(shown("0.23", &custom(".#", false, 0, 0, 0, false)), "0.2");
        assert_eq!(
            shown("23.000000000000004", &custom(".#", false, 0, 0, 0, false)),
            "23.0"
        );
        assert_eq!(
            shown("23.000000000000004", &custom("#.#", false, 0, 0, 0, false)),
            "23"
        );
        assert_eq!(
            shown("2345.67", &custom("#", false, 0, 0, 0, true)),
            "2,346"
        );
        assert_eq!(
            shown("0.23", &custom("#.0000", false, 0, 0, 4, false)),
            "0.2300"
        );
        // Complex formats: zero and space slots, padded decimals.
        assert_eq!(
            shown("2.34", &custom("000,000,000", true, 9, 9, 0, true)),
            "000,000,002"
        );
        assert_eq!(
            shown("2345.67", &custom("0,000,000", true, 0, 7, 0, true)),
            "    2,346"
        );
        assert_eq!(
            shown("0.23", &custom("0,000,000", true, 0, 7, 0, true)),
            "         "
        );
        assert_eq!(shown("2.34", &custom("00", true, 0, 2, 0, false)), " 2");
        assert_eq!(shown("0.23", &custom("#.0", true, 0, 0, 0, false)), ".2");
        assert_eq!(
            shown("0.23", &custom("#.0000", true, 0, 0, 0, false)),
            ".23  "
        );
        assert_eq!(
            shown("0.23", &custom(".0000", true, 0, 0, 0, false)),
            ".2300"
        );
        assert_eq!(
            shown("0.23", &custom("0,000.####", true, 0, 4, 0, false)),
            "  00.23"
        );
        assert_eq!(shown("0.23", &custom("0.##", true, 0, 1, 0, false)), " .23");
        assert_eq!(
            shown("0.23", &custom("000,000,000.0000000", true, 9, 9, 0, true)),
            "000,000,000.23     "
        );
    }

    #[test]
    fn formats_reach_excel_as_number_formats() {
        let currency = Format {
            kind: CURRENCY,
            currency_code: "GBP".to_string(),
            decimal_places: 2,
            show_thousands_separator: true,
            negative_style: 2,
            ..Format::default()
        };
        assert_eq!(
            excel(&currency, None),
            Excel::Code("\"£\"#,##0.00;(\"£\"#,##0.00)".to_string())
        );
        let percent = Format {
            kind: PERCENT,
            decimal_places: 1,
            ..Format::default()
        };
        assert_eq!(excel(&percent, None), Excel::Code("0.0%".to_string()));
        assert_eq!(
            excel(
                &Format {
                    kind: CHECKBOX,
                    ..Format::default()
                },
                None
            ),
            Excel::Boolean
        );
        assert_eq!(
            excel_date("EEE, d MMM yyyy HH:mm:ss").as_deref(),
            Some("ddd, d mmm yyyy hh:mm:ss")
        );
        assert_eq!(excel_date("h:mm a").as_deref(), Some("h:mm AM/PM"));
        assert_eq!(excel_date("'Week' w"), None);
        let units = CustomFormat {
            format: Format {
                custom_format_string: "#,##0.00' units'".to_string(),
                show_thousands_separator: true,
                scale_factor: 1.0,
                ..Format::default()
            },
            conditions: Vec::new(),
        };
        assert_eq!(
            excel(&Format::default(), Some(&units)),
            Excel::Code("#,##0.00\" units\"".to_string())
        );
    }

    #[test]
    fn fractions_and_bases() {
        let halves = Format {
            kind: FRACTION,
            fraction_accuracy: 2,
            ..Format::default()
        };
        assert_eq!(number(2.5, None, &halves, None), "2 1/2");
        let one_digit = Format {
            kind: FRACTION,
            fraction_accuracy: 0xFFFF_FFFF,
            ..Format::default()
        };
        assert_eq!(number(0.23, None, &one_digit, None), "2/9");
        assert_eq!(number(2.34, None, &one_digit, None), "2 1/3");
        let hex = Format {
            kind: BASE,
            base: 16,
            ..Format::default()
        };
        assert_eq!(number(255.0, None, &hex, None), "FF");
    }

    #[test]
    fn dates_follow_their_pattern() {
        // 2022-01-05 is 7,674 days after 2001-01-01.
        let seconds = 7_674.0 * 86_400.0;
        assert_eq!(
            date(seconds, "EEE, d MMM yyyy HH:mm:ss"),
            "Wed, 5 Jan 2022 00:00:00"
        );
        assert_eq!(date(seconds, "'Day #'DDD' of 'yyyy"), "Day #005 of 2022");
        assert_eq!(date(seconds + 13.5 * 3600.0, "h:mm a"), "1:30\u{202f}PM");
        assert_eq!(
            date(seconds + 13.5 * 3600.0, "d MMM h:mm a"),
            "5 Jan 1:30 PM"
        );
    }

    #[test]
    fn durations_in_their_units() {
        let short = Format {
            duration_style: 1,
            duration_unit_largest: WEEK,
            duration_unit_smallest: SECOND,
            ..Format::default()
        };
        assert_eq!(duration(694_861.0, &short), "1w 1d 1h 1m 1s");
        let compact = Format {
            duration_style: 0,
            duration_unit_largest: HOUR,
            duration_unit_smallest: SECOND,
            ..Format::default()
        };
        assert_eq!(duration(3_725.0, &compact), "1:02:05");
    }
}
