//! Scanners shared by the block and inline parsers: labels, destinations,
//! titles, entities, escapes, HTML tags, and autolinks.

use super::entities::lookup as lookup_entity;

pub const REPLACEMENT: char = '\u{FFFD}';

pub fn is_ascii_punctuation(byte: u8) -> bool {
    byte.is_ascii_punctuation()
}

/// Unicode whitespace per the spec: `Zs` plus tab, LF, FF, CR.
pub fn is_unicode_whitespace(ch: char) -> bool {
    ch == ' '
        || ch == '\t'
        || ch == '\n'
        || ch == '\u{0C}'
        || ch == '\r'
        || (ch.is_whitespace() && !ch.is_control())
}

/// Unicode punctuation per the spec: general categories P and S. The
/// standard library has no category lookup, so this treats every
/// non-alphanumeric, non-whitespace, non-control character as punctuation,
/// which also sweeps in marks and format characters (rare next to delimiter
/// runs).
pub fn is_unicode_punctuation(ch: char) -> bool {
    if ch.is_ascii() {
        return ch.is_ascii_punctuation();
    }
    !ch.is_alphanumeric() && !ch.is_whitespace() && !ch.is_control()
}

/// Case-folds and collapses interior whitespace, per the link label rules.
pub fn normalize_label(label: &str) -> String {
    let mut normalized = String::with_capacity(label.len());
    let mut pending_space = false;
    for ch in label.trim().chars() {
        if ch.is_whitespace() {
            pending_space = true;
            continue;
        }
        if pending_space {
            normalized.push(' ');
            pending_space = false;
        }
        for lowered in ch.to_lowercase() {
            // Full case folding differs from lowercasing for a few letters.
            match lowered {
                '\u{DF}' => normalized.push_str("ss"),
                '\u{17F}' => normalized.push('s'),
                '\u{3C2}' => normalized.push('\u{3C3}'),
                other => normalized.push(other),
            }
        }
    }
    normalized
}

/// Decodes the character reference starting at `text[0] == '&'`. Returns
/// the replacement and the number of bytes consumed.
pub fn decode_entity(text: &[u8]) -> Option<(String, usize)> {
    if text.first() != Some(&b'&') {
        return None;
    }
    if text.get(1) == Some(&b'#') {
        let (radix, digits_start) = if matches!(text.get(2), Some(b'x') | Some(b'X')) {
            (16u32, 3usize)
        } else {
            (10u32, 2usize)
        };
        let max_digits = if radix == 16 { 6 } else { 7 };
        let mut index = digits_start;
        let mut value: u32 = 0;
        let mut digits = 0usize;
        while let Some(byte) = text.get(index) {
            let digit = match (*byte as char).to_digit(radix) {
                Some(digit) => digit,
                None => break,
            };
            value = value.saturating_mul(radix).saturating_add(digit);
            digits += 1;
            index += 1;
            if digits > max_digits {
                return None;
            }
        }
        if digits == 0 || text.get(index) != Some(&b';') {
            return None;
        }
        let ch = match char::from_u32(value) {
            Some(ch) if value != 0 => ch,
            _ => REPLACEMENT,
        };
        return Some((ch.to_string(), index + 1));
    }
    let mut index = 1usize;
    while let Some(byte) = text.get(index) {
        if !byte.is_ascii_alphanumeric() {
            break;
        }
        index += 1;
        if index > 32 {
            return None;
        }
    }
    if index == 1 || text.get(index) != Some(&b';') {
        return None;
    }
    let name = std::str::from_utf8(&text[1..index]).ok()?;
    let replacement = lookup_entity(name)?;
    Some((replacement.to_string(), index + 1))
}

/// Resolves backslash escapes and character references.
pub fn unescape_and_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut index = 0usize;
    let mut segment_start = 0usize;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'\\' && index + 1 < bytes.len() && is_ascii_punctuation(bytes[index + 1]) {
            out.push_str(&text[segment_start..index]);
            out.push(bytes[index + 1] as char);
            index += 2;
            segment_start = index;
        } else if byte == b'&' {
            if let Some((replacement, consumed)) = decode_entity(&bytes[index..]) {
                out.push_str(&text[segment_start..index]);
                out.push_str(&replacement);
                index += consumed;
                segment_start = index;
            } else {
                index += 1;
            }
        } else {
            index += 1;
        }
    }
    out.push_str(&text[segment_start..]);
    out
}

/// A link label at `text[0] == '['`: up to 999 bytes of content with no
/// unescaped brackets. Returns the length including both brackets.
pub fn scan_link_label(text: &[u8]) -> Option<usize> {
    if text.first() != Some(&b'[') {
        return None;
    }
    let mut index = 1usize;
    while let Some(byte) = text.get(index) {
        match byte {
            b'\\' => index += 2,
            b'[' => return None,
            b']' => {
                if index - 1 > 999 {
                    return None;
                }
                return Some(index + 1);
            }
            _ => index += 1,
        }
    }
    None
}

/// A link destination at `text[0]`. Returns `(content start, content end,
/// consumed)`; the pointy-bracket form excludes its brackets from content.
pub fn scan_link_destination(text: &[u8]) -> Option<(usize, usize, usize)> {
    if text.first() == Some(&b'<') {
        let mut index = 1usize;
        while let Some(byte) = text.get(index) {
            match byte {
                b'\\' => index += 2,
                b'\n' | b'<' => return None,
                b'>' => return Some((1, index, index + 1)),
                _ => index += 1,
            }
        }
        return None;
    }
    let mut index = 0usize;
    let mut depth = 0usize;
    while let Some(byte) = text.get(index) {
        match byte {
            b'\\'
                if text
                    .get(index + 1)
                    .is_some_and(|next| is_ascii_punctuation(*next)) =>
            {
                index += 2
            }
            b'(' => {
                depth += 1;
                if depth > 32 {
                    return None;
                }
                index += 1;
            }
            b')' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
                index += 1;
            }
            byte if *byte <= b' ' => break,
            _ => index += 1,
        }
    }
    if depth != 0 {
        return None;
    }
    Some((0, index, index))
}

/// A link title at `text[0]`, delimited by `"`, `'`, or parentheses.
/// Returns `(content start, content end, consumed)`.
pub fn scan_link_title(text: &[u8]) -> Option<(usize, usize, usize)> {
    let open = *text.first()?;
    let close = match open {
        b'"' => b'"',
        b'\'' => b'\'',
        b'(' => b')',
        _ => return None,
    };
    let mut index = 1usize;
    while let Some(byte) = text.get(index) {
        match byte {
            b'\\'
                if text
                    .get(index + 1)
                    .is_some_and(|next| is_ascii_punctuation(*next)) =>
            {
                index += 2
            }
            byte if *byte == close => return Some((1, index, index + 1)),
            b'(' if open == b'(' => return None,
            _ => index += 1,
        }
    }
    None
}

fn skip_spaces_and_one_newline(text: &[u8], mut index: usize) -> usize {
    let mut seen_newline = false;
    while let Some(byte) = text.get(index) {
        match byte {
            b' ' | b'\t' => index += 1,
            b'\n' if !seen_newline => {
                seen_newline = true;
                index += 1;
            }
            _ => break,
        }
    }
    index
}

/// A link reference definition at the start of `text`. Returns the label,
/// decoded destination, decoded title, and bytes consumed through the end
/// of its last line.
pub fn parse_reference_definition(text: &str) -> Option<(String, String, String, usize)> {
    let bytes = text.as_bytes();
    let label_length = scan_link_label(bytes)?;
    let label = &text[1..label_length - 1];
    if label.trim().is_empty() {
        return None;
    }
    let mut index = label_length;
    if bytes.get(index) != Some(&b':') {
        return None;
    }
    index += 1;
    index = skip_spaces_and_one_newline(bytes, index);
    let (dest_start, dest_end, dest_consumed) = scan_link_destination(&bytes[index..])?;
    if dest_consumed == 0 && bytes.get(index) != Some(&b'<') {
        return None;
    }
    let destination = unescape_and_decode(&text[index + dest_start..index + dest_end]);
    index += dest_consumed;
    let after_destination = index;
    let before_title = skip_spaces_and_one_newline(bytes, index);
    let mut title = String::new();
    let mut end = after_destination;
    let had_whitespace = before_title > after_destination;
    if had_whitespace {
        if let Some((title_start, title_end, title_consumed)) =
            scan_link_title(&bytes[before_title..])
        {
            let after_title = before_title + title_consumed;
            if rest_of_line_is_blank(bytes, after_title) {
                title = unescape_and_decode(
                    &text[before_title + title_start..before_title + title_end],
                );
                end = after_title;
            }
        }
    }
    if end == after_destination && !rest_of_line_is_blank(bytes, after_destination) {
        return None;
    }
    let consumed = end_of_line(bytes, end);
    Some((label.to_string(), destination, title, consumed))
}

fn rest_of_line_is_blank(text: &[u8], mut index: usize) -> bool {
    while let Some(byte) = text.get(index) {
        match byte {
            b' ' | b'\t' => index += 1,
            b'\n' => return true,
            _ => return false,
        }
    }
    true
}

fn end_of_line(text: &[u8], mut index: usize) -> usize {
    while let Some(byte) = text.get(index) {
        index += 1;
        if *byte == b'\n' {
            break;
        }
    }
    index
}

// ----- HTML -----

const BLOCK_TAGS: &[&str] = &[
    "address",
    "article",
    "aside",
    "base",
    "basefont",
    "blockquote",
    "body",
    "caption",
    "center",
    "col",
    "colgroup",
    "dd",
    "details",
    "dialog",
    "dir",
    "div",
    "dl",
    "dt",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "frame",
    "frameset",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hr",
    "html",
    "iframe",
    "legend",
    "li",
    "link",
    "main",
    "menu",
    "menuitem",
    "nav",
    "noframes",
    "ol",
    "optgroup",
    "option",
    "p",
    "param",
    "search",
    "section",
    "summary",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "title",
    "tr",
    "track",
    "ul",
];

/// Tags the GFM tag filter neutralizes.
pub const FILTERED_TAGS: &[&str] = &[
    "title",
    "textarea",
    "style",
    "xmp",
    "iframe",
    "noembed",
    "noframes",
    "script",
    "plaintext",
];

fn starts_with_ignore_case(text: &[u8], prefix: &str) -> bool {
    let prefix = prefix.as_bytes();
    text.len() >= prefix.len() && text[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn tag_name_length(text: &[u8]) -> usize {
    let mut length = 0usize;
    while let Some(byte) = text.get(length) {
        if !(byte.is_ascii_alphanumeric() || *byte == b'-') {
            break;
        }
        length += 1;
    }
    length
}

/// The seven HTML block start conditions. `rest` starts at `<`.
pub fn scan_html_block_start(rest: &[u8], in_paragraph: bool) -> Option<u8> {
    if rest.first() != Some(&b'<') {
        return None;
    }
    for name in ["script", "pre", "style", "textarea"] {
        if starts_with_ignore_case(&rest[1..], name) {
            let after = rest.get(1 + name.len()).copied();
            if matches!(after, None | Some(b' ') | Some(b'\t') | Some(b'>')) {
                return Some(1);
            }
        }
    }
    if rest.starts_with(b"<!--") {
        return Some(2);
    }
    if rest.starts_with(b"<?") {
        return Some(3);
    }
    if rest.starts_with(b"<!") && rest.get(2).is_some_and(|byte| byte.is_ascii_alphabetic()) {
        return Some(4);
    }
    if rest.starts_with(b"<![CDATA[") {
        return Some(5);
    }
    let mut index = 1usize;
    if rest.get(index) == Some(&b'/') {
        index += 1;
    }
    let name_length = tag_name_length(&rest[index..]);
    if name_length > 0 {
        let name = std::str::from_utf8(&rest[index..index + name_length]).unwrap_or("");
        let lowered = name.to_ascii_lowercase();
        if BLOCK_TAGS.contains(&lowered.as_str()) {
            let after = rest.get(index + name_length).copied();
            let ok = match after {
                None | Some(b' ') | Some(b'\t') | Some(b'>') => true,
                Some(b'/') => rest.get(index + name_length + 1) == Some(&b'>'),
                _ => false,
            };
            if ok {
                return Some(6);
            }
        }
    }
    if in_paragraph {
        return None;
    }
    let tag_length = if rest.get(1) == Some(&b'/') {
        scan_closing_tag(rest)
    } else {
        scan_open_tag(rest)
    };
    if let Some(length) = tag_length {
        if rest_is_whitespace(&rest[length..]) {
            return Some(7);
        }
    }
    None
}

fn rest_is_whitespace(text: &[u8]) -> bool {
    text.iter().all(|byte| *byte == b' ' || *byte == b'\t')
}

/// Whether `line` ends an HTML block of `kind` (kinds 6 and 7 end on blank
/// lines, handled by the block parser).
pub fn scan_html_block_end(kind: u8, line: &[u8]) -> bool {
    let lowered = line.to_ascii_lowercase();
    let contains = |needle: &str| {
        lowered
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    match kind {
        1 => {
            contains("</script>")
                || contains("</pre>")
                || contains("</style>")
                || contains("</textarea>")
        }
        2 => contains("-->"),
        3 => contains("?>"),
        4 => contains(">"),
        5 => contains("]]>"),
        _ => false,
    }
}

fn scan_open_tag(text: &[u8]) -> Option<usize> {
    if text.first() != Some(&b'<') {
        return None;
    }
    let mut index = 1usize;
    if !text
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        return None;
    }
    index += tag_name_length(&text[index..]);
    loop {
        let mut whitespace = 0usize;
        while matches!(
            text.get(index + whitespace),
            Some(b' ') | Some(b'\t') | Some(b'\n')
        ) {
            whitespace += 1;
        }
        let after = index + whitespace;
        match text.get(after) {
            Some(b'>') => return Some(after + 1),
            Some(b'/') => {
                return if text.get(after + 1) == Some(&b'>') {
                    Some(after + 2)
                } else {
                    None
                };
            }
            Some(byte)
                if whitespace > 0
                    && (byte.is_ascii_alphabetic() || *byte == b'_' || *byte == b':') =>
            {
                index = after + 1;
                while text.get(index).is_some_and(|byte| {
                    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b':' | b'-')
                }) {
                    index += 1;
                }
                let mut value_index = index;
                while matches!(
                    text.get(value_index),
                    Some(b' ') | Some(b'\t') | Some(b'\n')
                ) {
                    value_index += 1;
                }
                if text.get(value_index) == Some(&b'=') {
                    value_index += 1;
                    while matches!(
                        text.get(value_index),
                        Some(b' ') | Some(b'\t') | Some(b'\n')
                    ) {
                        value_index += 1;
                    }
                    match text.get(value_index) {
                        Some(b'"') | Some(b'\'') => {
                            let quote = text[value_index];
                            value_index += 1;
                            while let Some(byte) = text.get(value_index) {
                                if *byte == quote {
                                    break;
                                }
                                value_index += 1;
                            }
                            if text.get(value_index) != Some(&quote) {
                                return None;
                            }
                            index = value_index + 1;
                        }
                        Some(byte)
                            if !matches!(
                                byte,
                                b' ' | b'\t' | b'\n' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`'
                            ) =>
                        {
                            while text.get(value_index).is_some_and(|byte| {
                                !matches!(
                                    byte,
                                    b' ' | b'\t' | b'\n' | b'"' | b'\'' | b'=' | b'<' | b'>' | b'`'
                                )
                            }) {
                                value_index += 1;
                            }
                            index = value_index;
                        }
                        _ => return None,
                    }
                }
            }
            _ => return None,
        }
    }
}

fn scan_closing_tag(text: &[u8]) -> Option<usize> {
    if !text.starts_with(b"</") {
        return None;
    }
    if !text.get(2).is_some_and(|byte| byte.is_ascii_alphabetic()) {
        return None;
    }
    let mut index = 2 + tag_name_length(&text[2..]);
    while matches!(text.get(index), Some(b' ') | Some(b'\t') | Some(b'\n')) {
        index += 1;
    }
    if text.get(index) == Some(&b'>') {
        Some(index + 1)
    } else {
        None
    }
}

/// Raw inline HTML at `text[0] == '<'`: open or closing tag, comment,
/// processing instruction, declaration, or CDATA. Returns the length.
pub fn scan_inline_html(text: &[u8]) -> Option<usize> {
    if text.first() != Some(&b'<') {
        return None;
    }
    if let Some(length) = scan_open_tag(text) {
        return Some(length);
    }
    if let Some(length) = scan_closing_tag(text) {
        return Some(length);
    }
    if text.starts_with(b"<!-->") {
        return Some(5);
    }
    if text.starts_with(b"<!--->") {
        return Some(6);
    }
    if text.starts_with(b"<!--") {
        let mut index = 4usize;
        while index + 3 <= text.len() {
            if &text[index..index + 3] == b"-->" {
                return Some(index + 3);
            }
            index += 1;
        }
        return None;
    }
    if text.starts_with(b"<?") {
        let mut index = 2usize;
        while index + 2 <= text.len() {
            if &text[index..index + 2] == b"?>" {
                return Some(index + 2);
            }
            index += 1;
        }
        return None;
    }
    if text.starts_with(b"<![CDATA[") {
        let mut index = 9usize;
        while index + 3 <= text.len() {
            if &text[index..index + 3] == b"]]>" {
                return Some(index + 3);
            }
            index += 1;
        }
        return None;
    }
    if text.starts_with(b"<!") && text.get(2).is_some_and(|byte| byte.is_ascii_alphabetic()) {
        let mut index = 3usize;
        while let Some(byte) = text.get(index) {
            if *byte == b'>' {
                return Some(index + 1);
            }
            index += 1;
        }
        return None;
    }
    None
}

/// Autolink at `text[0] == '<'`. Returns `(length, is_email)`.
pub fn scan_autolink(text: &[u8]) -> Option<(usize, bool)> {
    if text.first() != Some(&b'<') {
        return None;
    }
    // URI autolink: scheme of 2..=32 chars, then ':' then no spaces or brackets.
    let mut index = 1usize;
    if text
        .get(index)
        .is_some_and(|byte| byte.is_ascii_alphabetic())
    {
        let mut scheme_length = 0usize;
        while text
            .get(index + scheme_length)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-'))
        {
            scheme_length += 1;
        }
        if (2..=32).contains(&scheme_length) && text.get(index + scheme_length) == Some(&b':') {
            index += scheme_length + 1;
            while let Some(byte) = text.get(index) {
                match byte {
                    b'>' => return Some((index + 1, false)),
                    b'<' | b' ' | b'\t' | b'\n' => break,
                    byte if *byte < 0x20 || *byte == 0x7F => break,
                    _ => index += 1,
                }
            }
        }
    }
    // Email autolink.
    let mut index = 1usize;
    let mut local = 0usize;
    while text.get(index).is_some_and(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'.' | b'!'
                    | b'#'
                    | b'$'
                    | b'%'
                    | b'&'
                    | b'\''
                    | b'*'
                    | b'+'
                    | b'/'
                    | b'='
                    | b'?'
                    | b'^'
                    | b'_'
                    | b'`'
                    | b'{'
                    | b'|'
                    | b'}'
                    | b'~'
                    | b'-'
            )
    }) {
        index += 1;
        local += 1;
    }
    if local == 0 || text.get(index) != Some(&b'@') {
        return None;
    }
    index += 1;
    loop {
        let label_start = index;
        let mut label_length = 0usize;
        while text
            .get(index)
            .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'-')
        {
            index += 1;
            label_length += 1;
        }
        if label_length == 0 || label_length > 63 {
            return None;
        }
        if text[label_start] == b'-' || text[index - 1] == b'-' {
            return None;
        }
        match text.get(index) {
            Some(b'.') => index += 1,
            Some(b'>') => return Some((index + 1, true)),
            _ => return None,
        }
    }
}
