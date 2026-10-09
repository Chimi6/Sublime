//! Converter trait and the types every converter shares.

use std::fmt;
use std::fs::File;
use std::io::{self, Cursor, Read, Seek, SeekFrom, Write};

use crate::event::Context;
use crate::format::Format;

/// The declared fidelity contract of a converter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Fidelity {
    Lossless,
    /// Lossless for some inputs, lossy for others. The text says when.
    Conditional(&'static str),
    /// Always loses something. The text says what.
    Lossy(&'static str),
}

/// Fidelity without the description, ordered from best to worst.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum FidelityKind {
    Lossless,
    Conditional,
    Lossy,
}

impl FidelityKind {
    pub fn label(&self) -> &'static str {
        match self {
            FidelityKind::Lossless => "lossless",
            FidelityKind::Conditional => "conditional",
            FidelityKind::Lossy => "lossy",
        }
    }
}

impl Fidelity {
    /// Planner edge cost.
    pub fn cost(&self) -> u32 {
        match self {
            Fidelity::Lossless => 1,
            Fidelity::Conditional(_) => 10,
            Fidelity::Lossy(_) => 100,
        }
    }

    pub fn kind(&self) -> FidelityKind {
        match self {
            Fidelity::Lossless => FidelityKind::Lossless,
            Fidelity::Conditional(_) => FidelityKind::Conditional,
            Fidelity::Lossy(_) => FidelityKind::Lossy,
        }
    }

    pub fn is_lossless(&self) -> bool {
        self.kind() == FidelityKind::Lossless
    }

    pub fn description(&self) -> Option<&'static str> {
        match self {
            Fidelity::Lossless => None,
            Fidelity::Conditional(text) => Some(text),
            Fidelity::Lossy(text) => Some(text),
        }
    }
}

impl fmt::Display for Fidelity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = self.kind().label();
        match self.description() {
            Some(text) => write!(formatter, "{label}: {text}"),
            None => write!(formatter, "{label}"),
        }
    }
}

/// Where a converter's code comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Our own parser and writer.
    Native,
    /// A Rust crate compiled into the binary.
    Library,
    /// An installed program we shell out to.
    External,
}

impl Tier {
    pub fn rank(&self) -> u32 {
        match self {
            Tier::Native => 0,
            Tier::Library => 1,
            Tier::External => 2,
        }
    }

    pub fn label(&self) -> &'static str {
        match self {
            Tier::Native => "native",
            Tier::Library => "library",
            Tier::External => "external",
        }
    }
}

/// A position in the input. Converters document what line and column mean
/// for their format (for CSV: record line and field index).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Location {
    pub line: u64,
    pub column: u64,
}

impl fmt::Display for Location {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "line {}, column {}", self.line, self.column)
    }
}

/// A readable source that can restart from the beginning, or read from
/// any offset (a bottom-up BMP reads its rows from the end).
pub trait RewindableRead: Read + Send {
    fn rewind(&mut self) -> io::Result<()>;
    /// Moves to `position` bytes from the start.
    fn seek_to(&mut self, position: u64) -> io::Result<()>;
}

impl RewindableRead for File {
    fn rewind(&mut self) -> io::Result<()> {
        self.seek(SeekFrom::Start(0))?;
        Ok(())
    }

    fn seek_to(&mut self, position: u64) -> io::Result<()> {
        self.seek(SeekFrom::Start(position))?;
        Ok(())
    }
}

impl<T: AsRef<[u8]> + Send> RewindableRead for Cursor<T> {
    fn rewind(&mut self) -> io::Result<()> {
        self.set_position(0);
        Ok(())
    }

    fn seek_to(&mut self, position: u64) -> io::Result<()> {
        self.set_position(position);
        Ok(())
    }
}

/// The input handed to a converter. Files are `Rewindable`; stdin and the
/// pipes inside a multi-hop chain are `Stream`.
pub enum Input<'a> {
    Stream(&'a mut (dyn Read + Send)),
    Rewindable(&'a mut dyn RewindableRead),
}

impl Read for Input<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Input::Stream(reader) => reader.read(buffer),
            Input::Rewindable(reader) => reader.read(buffer),
        }
    }

    /// Forwarded so a file input can size the buffer from its length
    /// instead of growing it by doubling.
    fn read_to_end(&mut self, buffer: &mut Vec<u8>) -> io::Result<usize> {
        match self {
            Input::Stream(reader) => reader.read_to_end(buffer),
            Input::Rewindable(reader) => reader.read_to_end(buffer),
        }
    }

    fn read_to_string(&mut self, buffer: &mut String) -> io::Result<usize> {
        match self {
            Input::Stream(reader) => reader.read_to_string(buffer),
            Input::Rewindable(reader) => reader.read_to_string(buffer),
        }
    }
}

/// What a two-pass converter gets after asking for a rewindable input.
pub enum Rewound<'a> {
    Borrowed(&'a mut dyn RewindableRead),
    Buffered(Cursor<Vec<u8>>),
}

impl Read for Rewound<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        match self {
            Rewound::Borrowed(reader) => reader.read(buffer),
            Rewound::Buffered(cursor) => cursor.read(buffer),
        }
    }
}

impl RewindableRead for Rewound<'_> {
    fn rewind(&mut self) -> io::Result<()> {
        match self {
            Rewound::Borrowed(reader) => reader.rewind(),
            Rewound::Buffered(cursor) => RewindableRead::rewind(cursor),
        }
    }

    fn seek_to(&mut self, position: u64) -> io::Result<()> {
        match self {
            Rewound::Borrowed(reader) => reader.seek_to(position),
            Rewound::Buffered(cursor) => cursor.seek_to(position),
        }
    }
}

impl<'a> Input<'a> {
    /// Returns the input as something rewindable. A `Stream` is read fully
    /// into memory once; a `Rewindable` is borrowed as is.
    pub fn into_rewindable(self) -> io::Result<Rewound<'a>> {
        match self {
            Input::Rewindable(reader) => Ok(Rewound::Borrowed(reader)),
            Input::Stream(reader) => {
                let mut buffer = Vec::new();
                reader.read_to_end(&mut buffer)?;
                let cursor = Cursor::new(buffer);
                Ok(Rewound::Buffered(cursor))
            }
        }
    }
}

#[derive(Debug)]
pub enum ConvertError {
    Io(io::Error),
    Malformed { location: Location, message: String },
    Unsupported(String),
}

impl From<io::Error> for ConvertError {
    fn from(error: io::Error) -> Self {
        ConvertError::Io(error)
    }
}

impl fmt::Display for ConvertError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ConvertError::Io(error) => write!(formatter, "{error}"),
            ConvertError::Malformed { location, message } => {
                write!(formatter, "{location}: {message}")
            }
            ConvertError::Unsupported(message) => write!(formatter, "{message}"),
        }
    }
}

impl std::error::Error for ConvertError {}

/// Options that apply to a whole conversion.
#[derive(Debug, Clone, Default)]
pub struct ConvertOptions {
    pub strict: bool,
    /// The worksheet to read or the name to give the one written: a sheet
    /// name or a 1-based number; the first sheet when absent.
    pub sheet: Option<String>,
    /// 1 to 100: the quality a lossy image writer encodes at (85 for
    /// JPEG when absent), or a lossless writer's effort (WebP: 50 and
    /// under fastest, 90 and up smallest).
    pub quality: Option<u8>,
    /// The page to read from a paged document (PDF): 1-based, the first
    /// when absent.
    pub page: Option<u32>,
    /// A font file (TrueType, `.ttf`) to set a written document's body
    /// text in (PDF); the standard fonts, with the machine's fonts for
    /// other scripts, when absent.
    pub font: Option<std::sync::Arc<Vec<u8>>>,
    /// The delimiter of comma-separated text (CSV) read and written: a
    /// semicolon or pipe; read input's own (from its first records) and a
    /// comma when absent.
    pub delimiter: Option<u8>,
}

/// One edge in the format graph. Implement this and add one line to
/// `src/registry.rs`.
/// Where a converter that splits its input (a workbook's sheets, a
/// document's tables, a PDF's pages) writes each part.
pub trait Parts {
    /// Starts the part named `name` (a sheet's name, `Page 3`); what is
    /// written to the returned writer until the next call is that part.
    fn part(&mut self, name: &str) -> io::Result<&mut dyn Write>;
}

pub trait Converter: Sync {
    fn name(&self) -> &'static str;
    fn from(&self) -> &'static Format;
    fn to(&self) -> &'static Format;
    fn fidelity(&self) -> Fidelity;
    fn tier(&self) -> Tier;
    fn convert(
        &self,
        input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError>;

    /// Whether `convert_parts` writes each part of a many-part input (every
    /// sheet, table, or page) as its own output, where `convert` writes
    /// only the first.
    fn splits(&self) -> bool {
        false
    }

    /// Writes the input's parts, each through `parts`. Converters that do
    /// not split write one unnamed part.
    fn convert_parts(
        &self,
        input: Input<'_>,
        parts: &mut dyn Parts,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let output = parts.part("")?;
        self.convert(input, output, context)
    }
}

/// A part's file name: its name made safe for a file system, `Part N` when
/// it has none, and numbered when an earlier part took it.
pub fn part_file_stem(name: &str, index: usize, taken: &[String]) -> String {
    let cleaned: String = name
        .chars()
        .map(|character| match character {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            character if character.is_control() => '_',
            character => character,
        })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    let base = if cleaned.is_empty() {
        format!("Part {}", index + 1)
    } else {
        cleaned
    };
    let mut candidate = base.clone();
    let mut number = 2;
    while taken
        .iter()
        .any(|taken| taken.eq_ignore_ascii_case(&candidate))
    {
        candidate = format!("{base} ({number})");
        number += 1;
    }
    candidate
}

/// Parts gathered in memory, in order, each with its name.
#[derive(Debug, Default)]
pub struct MemoryParts {
    pub parts: Vec<(String, Vec<u8>)>,
}

impl MemoryParts {
    /// The parts as one ZIP, each a file named after its part with
    /// `extension`, as the command line names them.
    pub fn zip(&self, extension: &str) -> io::Result<Vec<u8>> {
        let mut zip = crate::io::zip::ZipWriter::new(Vec::new());
        let mut names: Vec<String> = Vec::new();
        for (index, (name, bytes)) in self.parts.iter().enumerate() {
            let stem = part_file_stem(name, index, &names);
            zip.add_deflated(&format!("{stem}.{extension}"), bytes)?;
            names.push(stem);
        }
        zip.finish()
    }
}

impl Parts for MemoryParts {
    fn part(&mut self, name: &str) -> io::Result<&mut dyn Write> {
        self.parts.push((name.to_string(), Vec::new()));
        let last = self.parts.len() - 1;
        Ok(&mut self.parts[last].1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn fidelity_costs_follow_the_spec() {
        assert_eq!(Fidelity::Lossless.cost(), 1);
        assert_eq!(Fidelity::Conditional("x").cost(), 10);
        assert_eq!(Fidelity::Lossy("y").cost(), 100);
    }

    #[test]
    fn fidelity_kinds_order_from_best_to_worst() {
        assert!(FidelityKind::Lossless < FidelityKind::Conditional);
        assert!(FidelityKind::Conditional < FidelityKind::Lossy);
        assert_eq!(Fidelity::Lossy("y").kind(), FidelityKind::Lossy);
        assert_eq!(FidelityKind::Conditional.label(), "conditional");
    }

    #[test]
    fn tier_ranks_native_first() {
        assert!(Tier::Native.rank() < Tier::Library.rank());
        assert!(Tier::Library.rank() < Tier::External.rank());
        assert_eq!(Tier::External.label(), "external");
    }

    #[test]
    fn stream_input_buffers_when_made_rewindable() {
        let mut source: &[u8] = b"hello";
        let input = Input::Stream(&mut source);
        let mut rewound = input.into_rewindable().unwrap();
        let mut first = String::new();
        rewound.read_to_string(&mut first).unwrap();
        rewound.rewind().unwrap();
        let mut second = String::new();
        rewound.read_to_string(&mut second).unwrap();
        assert_eq!(first, "hello");
        assert_eq!(second, "hello");
    }

    #[test]
    fn rewindable_input_is_borrowed_not_copied() {
        let mut cursor = Cursor::new(b"abc".to_vec());
        let input = Input::Rewindable(&mut cursor);
        let rewound = input.into_rewindable().unwrap();
        let is_borrowed = matches!(rewound, Rewound::Borrowed(_));
        assert!(is_borrowed);
    }

    #[test]
    fn input_reads_transparently() {
        let mut source: &[u8] = b"xyz";
        let mut input = Input::Stream(&mut source);
        let mut text = String::new();
        input.read_to_string(&mut text).unwrap();
        assert_eq!(text, "xyz");
    }

    #[test]
    fn convert_error_displays_location() {
        let error = ConvertError::Malformed {
            location: Location { line: 3, column: 7 },
            message: "bad".to_string(),
        };
        assert_eq!(error.to_string(), "line 3, column 7: bad");
    }
}
