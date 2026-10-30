//! Runs a page's content stream for its text: the graphics state (`q`,
//! `Q`, `cm`), the text state and matrices (`BT` to `ET`, `Tc Tw Tz TL
//! Tf Ts`, `Td TD Tm T*`), and the text-showing operators (`Tj TJ ' "`),
//! into glyphs placed in page space. Form XObjects are run with their
//! matrix and resources; inline images are skipped; everything that
//! paints is ignored. Marked content with `/ActualText` (a ligature, a
//! hyphen, or a glyph drawn for a character the font lacks) reads as that
//! text in place of its glyphs', as viewers copy it.

use std::rc::Rc;

use crate::io::pdf::document::{Document, find};
use crate::io::pdf::font::{Code, Font, FontCache};
use crate::io::pdf::lexer::{Lexer, Token};
use crate::io::pdf::object::{Dictionary, Object};

/// One shown glyph: where it starts and ends on its baseline, in page
/// space (points, y up), its size there, and its text in `PageText::text`.
/// A glyph of a vertical font starts at `y` and ends at `end` down the
/// page instead, `x` its column.
#[derive(Debug, Clone)]
pub struct Glyph {
    pub x: f64,
    pub end: f64,
    pub y: f64,
    pub size: f64,
    pub text: std::ops::Range<usize>,
    pub bold: bool,
    pub italic: bool,
    /// The text runs left to right along the x axis (not rotated), or
    /// down the page for a vertical font.
    pub upright: bool,
    /// Set by a vertical font (a `-V` CMap): it advances down the page.
    pub vertical: bool,
}

#[derive(Default)]
pub struct PageText {
    pub glyphs: Vec<Glyph>,
    pub text: String,
}

type Matrix = [f64; 6];

const IDENTITY: Matrix = [1.0, 0.0, 0.0, 1.0, 0.0, 0.0];

/// `first` then `second`: the product as PDF composes transforms.
fn multiply(first: &Matrix, second: &Matrix) -> Matrix {
    [
        first[0] * second[0] + first[1] * second[2],
        first[0] * second[1] + first[1] * second[3],
        first[2] * second[0] + first[3] * second[2],
        first[2] * second[1] + first[3] * second[3],
        first[4] * second[0] + first[5] * second[2] + second[4],
        first[4] * second[1] + first[5] * second[3] + second[5],
    ]
}

fn translate(x: f64, y: f64) -> Matrix {
    [1.0, 0.0, 0.0, 1.0, x, y]
}

/// The state `q` saves and `Q` restores.
#[derive(Clone)]
struct State {
    ctm: Matrix,
    char_spacing: f64,
    word_spacing: f64,
    scale: f64,
    leading: f64,
    font: Option<Rc<Font>>,
    size: f64,
    rise: f64,
}

/// How deep form XObjects may nest.
const FORM_DEPTH: usize = 8;

pub struct Interpreter<'d, 'a> {
    document: &'d Document<'a>,
    fonts: &'d mut FontCache,
    out: PageText,
    codes: Vec<Code>,
    /// Open marked-content sequences, innermost last: the replacement
    /// text of those with `/ActualText`.
    marked: Vec<Option<ActualText>>,
}

/// A span's replacement text, and whether it has been shown yet.
struct ActualText {
    text: String,
    shown: bool,
}

impl<'d, 'a> Interpreter<'d, 'a> {
    pub fn new(document: &'d Document<'a>, fonts: &'d mut FontCache) -> Interpreter<'d, 'a> {
        Interpreter {
            document,
            fonts,
            out: PageText::default(),
            codes: Vec::new(),
            marked: Vec::new(),
        }
    }

    /// The glyphs of one content stream run with `resources`.
    pub fn run(mut self, content: &[u8], resources: Option<&Dictionary>) -> PageText {
        let state = State {
            ctm: IDENTITY,
            char_spacing: 0.0,
            word_spacing: 0.0,
            scale: 1.0,
            leading: 0.0,
            font: None,
            size: 0.0,
            rise: 0.0,
        };
        self.stream(content, resources, state, 0);
        self.out
    }

    fn stream(
        &mut self,
        content: &[u8],
        resources: Option<&Dictionary>,
        start: State,
        depth: usize,
    ) {
        let mut state = start;
        let mut saved: Vec<State> = Vec::new();
        let mut text_matrix = IDENTITY;
        let mut line_matrix = IDENTITY;
        let mut operands: Vec<Object> = Vec::new();
        let mut lexer = Lexer::new(content);
        while let Some(token) = lexer.next_token() {
            let operator = match token {
                Token::Operand(object) => {
                    // A runaway operand list (broken content) is dropped.
                    if operands.len() < 64 {
                        operands.push(object);
                    }
                    continue;
                }
                Token::Operator(operator) => operator,
            };
            let number = |index: usize| {
                operands
                    .get(index)
                    .and_then(Object::as_number)
                    .unwrap_or(0.0)
            };
            let matrix = || -> Matrix {
                [
                    number(0),
                    number(1),
                    number(2),
                    number(3),
                    number(4),
                    number(5),
                ]
            };
            match operator {
                b"q" => {
                    if saved.len() < 64 {
                        saved.push(state.clone());
                    }
                }
                b"Q" => {
                    if let Some(previous) = saved.pop() {
                        state = previous;
                    }
                }
                b"cm" if operands.len() >= 6 => state.ctm = multiply(&matrix(), &state.ctm),
                b"BT" => {
                    text_matrix = IDENTITY;
                    line_matrix = IDENTITY;
                }
                b"Tc" => state.char_spacing = number(0),
                b"Tw" => state.word_spacing = number(0),
                b"Tz" => state.scale = number(0) / 100.0,
                b"TL" => state.leading = number(0),
                b"Ts" => state.rise = number(0),
                b"Tf" if operands.len() >= 2 => {
                    let name = operands[0].as_name().unwrap_or(b"");
                    state.font = self
                        .fonts
                        .get(self.document, resources, name)
                        .ok()
                        .flatten();
                    state.size = number(1);
                }
                b"Td" | b"TD" if operands.len() >= 2 => {
                    if operator == b"TD" {
                        state.leading = -number(1);
                    }
                    line_matrix = multiply(&translate(number(0), number(1)), &line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tm" if operands.len() >= 6 => {
                    line_matrix = matrix();
                    text_matrix = line_matrix;
                }
                b"T*" => {
                    line_matrix = multiply(&translate(0.0, -state.leading), &line_matrix);
                    text_matrix = line_matrix;
                }
                b"Tj" => {
                    if let Some(Object::String(bytes)) = operands.first() {
                        self.show(&state, &mut text_matrix, bytes);
                    }
                }
                b"'" | b"\"" => {
                    if operator == b"\"" && operands.len() >= 3 {
                        state.word_spacing = number(0);
                        state.char_spacing = number(1);
                    }
                    line_matrix = multiply(&translate(0.0, -state.leading), &line_matrix);
                    text_matrix = line_matrix;
                    if let Some(Object::String(bytes)) = operands.last() {
                        self.show(&state, &mut text_matrix, bytes);
                    }
                }
                b"TJ" => {
                    if let Some(Object::Array(items)) = operands.first() {
                        for item in items {
                            match item {
                                Object::String(bytes) => self.show(&state, &mut text_matrix, bytes),
                                other => {
                                    if let Some(adjust) = other.as_number() {
                                        let shift = -adjust / 1000.0 * state.size * state.scale;
                                        text_matrix =
                                            multiply(&translate(shift, 0.0), &text_matrix);
                                    }
                                }
                            }
                        }
                    }
                }
                b"Do" if depth < FORM_DEPTH => {
                    if let Some(name) = operands.first().and_then(Object::as_name) {
                        let name = name.to_vec();
                        self.form(&name, resources, &state, depth);
                    }
                }
                b"BI" => skip_inline_image(&mut lexer),
                b"BMC" => self.marked.push(None),
                b"BDC" => {
                    let properties = match operands.get(1) {
                        Some(Object::Dictionary(dictionary)) => Some(dictionary.clone()),
                        Some(Object::Name(name)) => resources
                            .and_then(|resources| resources.get(b"Properties"))
                            .and_then(|object| self.document.resolve(object).ok())
                            .and_then(|object| object.as_dictionary().cloned())
                            .and_then(|all| all.get(name).cloned())
                            .and_then(|object| self.document.resolve(&object).ok())
                            .and_then(|object| object.as_dictionary().cloned()),
                        _ => None,
                    };
                    let actual = properties
                        .and_then(|properties| properties.get(b"ActualText").cloned())
                        .and_then(|object| match object {
                            Object::String(bytes) => Some(ActualText {
                                text: text_string(&bytes),
                                shown: false,
                            }),
                            _ => None,
                        });
                    if self.marked.len() < 64 {
                        self.marked.push(actual);
                    }
                }
                b"EMC" => {
                    self.marked.pop();
                }
                _ => {}
            }
            operands.clear();
        }
    }

    /// Runs a form XObject named in `resources`.
    fn form(&mut self, name: &[u8], resources: Option<&Dictionary>, state: &State, depth: usize) {
        let document = self.document;
        let Some(object) = resources
            .and_then(|resources| resources.get(b"XObject"))
            .and_then(|objects| document.resolve(objects).ok())
            .and_then(|objects| {
                objects
                    .as_dictionary()
                    .and_then(|objects| objects.get(name))
                    .cloned()
            })
        else {
            return;
        };
        let Ok(Object::Stream(dictionary, _)) = document.resolve(&object) else {
            return;
        };
        if dictionary.get(b"Subtype").and_then(Object::as_name) != Some(b"Form") {
            return;
        }
        let Ok((_, content)) = document.stream_data(&object) else {
            return;
        };
        let form_matrix = dictionary
            .get(b"Matrix")
            .and_then(|object| document.resolve(object).ok())
            .and_then(|object| {
                let items = object.as_array()?;
                let values: Vec<f64> = items.iter().filter_map(Object::as_number).collect();
                (values.len() == 6).then(|| {
                    [
                        values[0], values[1], values[2], values[3], values[4], values[5],
                    ]
                })
            })
            .unwrap_or(IDENTITY);
        let own = dictionary
            .get(b"Resources")
            .and_then(|object| document.resolve(object).ok())
            .and_then(|object| object.as_dictionary().cloned());
        let mut inner = state.clone();
        inner.ctm = multiply(&form_matrix, &state.ctm);
        let resources = own.as_ref().or(resources);
        self.stream(&content, resources, inner, depth + 1);
    }

    /// Shows a string: each code becomes a glyph at the text position,
    /// which then advances by the code's width and the spacings.
    fn show(&mut self, state: &State, text_matrix: &mut Matrix, bytes: &[u8]) {
        let Some(font) = state.font.clone() else {
            return;
        };
        let mut codes = std::mem::take(&mut self.codes);
        font.codes(bytes, &mut codes);
        for code in &codes {
            let render = multiply(
                &[
                    state.size * state.scale,
                    0.0,
                    0.0,
                    state.size,
                    0.0,
                    state.rise,
                ],
                &multiply(text_matrix, &state.ctm),
            );
            if font.vertical {
                // Down by a full em (the default vertical displacement),
                // with the spacings as the specification adds them.
                let mut advance = -state.size + state.char_spacing;
                if code.word_space {
                    advance += state.word_spacing;
                }
                *text_matrix = multiply(&translate(0.0, advance), text_matrix);
            } else {
                let mut advance = code.advance * state.size + state.char_spacing;
                if code.word_space {
                    advance += state.word_spacing;
                }
                *text_matrix = multiply(&translate(advance * state.scale, 0.0), text_matrix);
            }
            let after = multiply(
                &[
                    state.size * state.scale,
                    0.0,
                    0.0,
                    state.size,
                    0.0,
                    state.rise,
                ],
                &multiply(text_matrix, &state.ctm),
            );
            let start = self.out.text.len();
            // Inside `/ActualText` the span's text stands for its glyphs:
            // written once, at the first glyph.
            if let Some(actual) = self.marked.iter_mut().rev().flatten().next() {
                if actual.shown {
                    continue;
                }
                actual.shown = true;
                self.out.text.push_str(&actual.text);
            } else {
                font.text(code.code, &mut self.out.text);
            }
            if self.out.text.len() == start {
                continue;
            }
            let size = render[2].hypot(render[3]) * font.size_factor;
            self.out.glyphs.push(Glyph {
                x: render[4],
                end: if font.vertical { after[5] } else { after[4] },
                y: render[5],
                size,
                text: start..self.out.text.len(),
                bold: font.bold,
                italic: font.italic,
                upright: render[1].abs() < 1e-6 * size.max(1.0) && render[0] > 0.0,
                vertical: font.vertical,
            });
        }
        self.codes = codes;
    }
}

/// A PDF text string: UTF-16 after its byte-order mark, UTF-8 after its
/// (PDF 2.0), and otherwise PDFDocEncoding, read here as Latin-1.
fn text_string(bytes: &[u8]) -> String {
    if let Some(units) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        let units = units
            .chunks(2)
            .map(|pair| u16::from_be_bytes([pair[0], *pair.get(1).unwrap_or(&0)]));
        return char::decode_utf16(units)
            .map(|unit| unit.unwrap_or('\u{FFFD}'))
            .collect();
    }
    if let Some(text) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(text).into_owned();
    }
    bytes.iter().map(|byte| char::from(*byte)).collect()
}

/// Moves past an inline image: its dictionary, `ID`, the data, and `EI`
/// standing between whitespace.
fn skip_inline_image(lexer: &mut Lexer<'_>) {
    while let Some(token) = lexer.next_token() {
        if let Token::Operator(b"ID") = token {
            break;
        }
    }
    let bytes = lexer.bytes();
    let mut at = lexer.position() + 1;
    while let Some(found) = find(bytes, b"EI", at) {
        let before = found > 0 && bytes[found - 1].is_ascii_whitespace();
        let after = bytes
            .get(found + 2)
            .is_none_or(|byte| byte.is_ascii_whitespace());
        if before && after {
            lexer.set_position(found + 2);
            return;
        }
        at = found + 2;
    }
    lexer.set_position(bytes.len());
}
