//! Reads an OpenDocument spreadsheet: its data styles and cell styles
//! (`styles.xml`, then `content.xml`'s automatic styles), then each sheet
//! (`table:table`) as rows of cells, each with the text LibreOffice shows
//! for it (as its CSV export writes it), its value as stored, and its
//! number format.
//!
//! `content.xml` is inflated a piece at a time and read once: each sheet
//! the caller wants is gathered compactly (its cells' text, and their
//! values and formats when asked for) and handed out whole, padded to its
//! extent (the last row and column with content, as LibreOffice bounds
//! its export); the sheets it does not want are passed over unparsed.
//! Repeated rows and cells (`number-rows-repeated`,
//! `number-columns-repeated`) are held once, so the empty ones that run to
//! a sheet's edge cost nothing. A sheet costs about its text, never the
//! XML it came in.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use super::numfmt::{self, DataStyle, OdsStyles};
use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum OdsError {
    Zip(ZipError),
    /// A part the package needs is missing or not UTF-8.
    Part(&'static str),
    /// The package is encrypted (a password-protected spreadsheet).
    Encrypted,
    NoSuchSheet {
        wanted: String,
        available: Vec<String>,
    },
}

impl fmt::Display for OdsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            OdsError::Zip(error) => write!(formatter, "not a spreadsheet package: {error}"),
            OdsError::Part(name) => write!(formatter, "the spreadsheet has no readable {name}"),
            OdsError::Encrypted => {
                write!(formatter, "the spreadsheet is password-protected")
            }
            OdsError::NoSuchSheet { wanted, available } => write!(
                formatter,
                "no sheet '{wanted}' (the sheets are: {})",
                available.join(", ")
            ),
        }
    }
}

impl std::error::Error for OdsError {}

impl From<ZipError> for OdsError {
    fn from(error: ZipError) -> Self {
        OdsError::Zip(error)
    }
}

/// A cell's value as stored.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Value {
    Empty,
    Text,
    Number(f64),
    /// Days since 1899-12-30, with the time of day as the fraction.
    Date(f64),
    /// A duration in days.
    Time(f64),
    Boolean(bool),
}

/// A cell as a sheet hands it out.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct OdsCell<'a> {
    /// The text LibreOffice shows for the cell.
    pub text: &'a str,
    /// The value as text: numbers as stored, dates and times in ISO 8601,
    /// booleans as `TRUE` and `FALSE`, otherwise the shown text (only
    /// when read with `Need::Typed`).
    pub raw: &'a str,
    pub value: Value,
    /// The cell's data style (`Spreadsheet::format`), when it has one
    /// (only when read with `Need::Typed`).
    pub format: Option<usize>,
}

const EMPTY: OdsCell<'static> = OdsCell {
    text: "",
    raw: "",
    value: Value::Empty,
    format: None,
};

/// What a sheet keeps of each cell: the shown text alone (for text
/// outputs), or its value and format too.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Need {
    Text,
    Typed,
}

/// The most rows and columns LibreOffice keeps; content past them is
/// dropped on loading, so it is not read.
const MAX_ROWS: usize = 1_048_576;
const MAX_COLUMNS: usize = 16_384;

/// How much of `content.xml` is inflated at a time, and how large a row
/// may grow before the reader parses inside it, at a cell's end.
const PIECE: usize = 4 << 20;
const ROW_LIMIT: usize = 64 << 20;

/// A cell being read.
struct Scratch {
    text: String,
    raw: String,
    value: Value,
    format: Option<usize>,
    /// Whether the cell holds a formula's error (shown as text).
    error: bool,
}

impl Scratch {
    fn new() -> Scratch {
        Scratch {
            text: String::new(),
            raw: String::new(),
            value: Value::Empty,
            format: None,
            error: false,
        }
    }

    fn has_content(&self) -> bool {
        self.value != Value::Empty || !self.text.is_empty()
    }
}

/// A cell's value and format as a typed sheet keeps them.
#[derive(Clone, Copy)]
struct Typed {
    raw_end: u32,
    format: u32,
    value: Value,
}

/// A sheet read whole: its cells' text in one buffer, each cell run (a
/// cell and its repeat) by where its text ends, each row run by where its
/// cells end.
pub struct Sheet {
    pub index: usize,
    pub name: String,
    text: String,
    raw: String,
    /// Per cell run: its text's end and how many columns it repeats over.
    cells: Vec<(u32, u32)>,
    typed: Vec<Typed>,
    /// Per row run: how many rows it repeats over and its cells' end.
    rows: Vec<(u32, u32)>,
    width: usize,
    height: usize,
    merges: Vec<(usize, usize, usize, usize)>,
}

impl Sheet {
    /// The rows and columns up to the sheet's last content.
    pub fn extent(&self) -> (usize, usize) {
        (self.height, self.width)
    }

    /// The merged ranges as (row, column, rows, columns) from 0.
    pub fn merges(&self) -> &[(usize, usize, usize, usize)] {
        &self.merges
    }

    /// Hands every row, padded to the sheet's width, to `on_row`.
    pub fn rows<E>(
        &self,
        on_row: &mut dyn FnMut(&[OdsCell<'_>]) -> Result<(), E>,
    ) -> Result<(), E> {
        let mut row: Vec<OdsCell<'_>> = Vec::with_capacity(self.width);
        let mut emitted = 0usize;
        let mut first = 0usize;
        for &(repeat, cells_end) in &self.rows {
            if emitted >= self.height {
                break;
            }
            row.clear();
            let cells_end = cells_end as usize;
            for index in first..cells_end {
                if row.len() >= self.width {
                    break;
                }
                let (end, repeat) = self.cells[index];
                let start = if index == 0 {
                    0
                } else {
                    self.cells[index - 1].0 as usize
                };
                let text = &self.text[start..end as usize];
                let cell = match self.typed.get(index) {
                    Some(typed) => {
                        let raw_start = if index == 0 {
                            0
                        } else {
                            self.typed[index - 1].raw_end as usize
                        };
                        OdsCell {
                            text,
                            raw: &self.raw[raw_start..typed.raw_end as usize],
                            value: typed.value,
                            format: (typed.format != u32::MAX).then_some(typed.format as usize),
                        }
                    }
                    None => OdsCell {
                        text,
                        raw: text,
                        value: if text.is_empty() {
                            Value::Empty
                        } else {
                            Value::Text
                        },
                        format: None,
                    },
                };
                let room = self.width - row.len();
                row.extend(std::iter::repeat_n(cell, (repeat as usize).min(room)));
            }
            first = cells_end;
            row.resize(self.width, EMPTY);
            let times = (repeat as usize).min(self.height - emitted);
            for _ in 0..times {
                on_row(&row)?;
            }
            emitted += times;
        }
        Ok(())
    }
}

/// A sheet being read.
struct Building {
    sheet: Sheet,
    need: Need,
    /// Column default cell styles, as runs (end column, style).
    columns: Vec<(usize, Option<String>)>,
    row_repeat: usize,
    row_style: Option<String>,
    /// The cell run where the open row's cells start.
    row_first: usize,
    column: usize,
    row: usize,
    /// The open row's content's end column.
    row_width: usize,
}

impl Building {
    fn new(index: usize, name: String, need: Need) -> Building {
        Building {
            sheet: Sheet {
                index,
                name,
                text: String::new(),
                raw: String::new(),
                cells: Vec::new(),
                typed: Vec::new(),
                rows: Vec::new(),
                width: 0,
                height: 0,
                merges: Vec::new(),
            },
            need,
            columns: Vec::new(),
            row_repeat: 1,
            row_style: None,
            row_first: 0,
            column: 0,
            row: 0,
            row_width: 0,
        }
    }

    /// Adds a cell run to the open row.
    fn push(&mut self, cell: &Scratch, repeat: usize, spans: (usize, usize)) {
        let (columns, rows) = spans;
        if columns > 1 || rows > 1 {
            self.sheet
                .merges
                .push((self.row, self.column, rows.max(1), columns.max(1)));
        }
        if self.row < MAX_ROWS && self.column < MAX_COLUMNS {
            if cell.has_content() {
                self.row_width = (self.column + repeat).min(MAX_COLUMNS);
            }
            let repeat = repeat.min(MAX_COLUMNS - self.column) as u32;
            let sheet = &mut self.sheet;
            sheet.text.push_str(&cell.text);
            sheet.cells.push((sheet.text.len() as u32, repeat));
            if self.need == Need::Typed {
                sheet.raw.push_str(&cell.raw);
                sheet.typed.push(Typed {
                    raw_end: sheet.raw.len() as u32,
                    format: cell.format.map_or(u32::MAX, |format| format as u32),
                    value: cell.value,
                });
            }
        }
        self.column += repeat;
    }

    fn end_row(&mut self) {
        let sheet = &mut self.sheet;
        // Trailing empty cells are padding, added back when handed out.
        while sheet.cells.len() > self.row_first {
            let last = sheet.cells.len() - 1;
            let start = if last == 0 {
                0
            } else {
                sheet.cells[last - 1].0
            };
            let empty = sheet.cells[last].0 == start
                && sheet
                    .typed
                    .get(last)
                    .is_none_or(|typed| typed.value == Value::Empty);
            if !empty {
                break;
            }
            sheet.cells.pop();
            sheet.typed.pop();
        }
        if self.row < MAX_ROWS {
            let repeat = self.row_repeat.min(MAX_ROWS - self.row) as u32;
            // An empty row after an empty row run joins it.
            let empty = sheet.cells.len() == self.row_first;
            let previous_empty = match sheet.rows.as_slice() {
                [.., (_, before), (_, last)] => before == last,
                [(_, last)] => *last == 0,
                [] => false,
            };
            match sheet.rows.last_mut() {
                Some((times, _)) if empty && previous_empty => *times += repeat,
                _ => sheet.rows.push((repeat, sheet.cells.len() as u32)),
            }
            if self.row_width > 0 {
                sheet.width = sheet.width.max(self.row_width);
                sheet.height = (self.row + self.row_repeat).min(MAX_ROWS);
            }
        }
        self.row += self.row_repeat;
        self.column = 0;
        self.row_width = 0;
        self.row_first = sheet.cells.len();
    }

    fn finish(mut self) -> Sheet {
        let (height, width) = (self.sheet.height, self.sheet.width);
        // Merged ranges count where they start inside the content.
        self.sheet
            .merges
            .retain(|(top, left, _, _)| *top < height && *left < width);
        self.sheet
    }
}

pub struct Spreadsheet<'a> {
    archive: ZipArchive<'a>,
    /// Data styles in the order cells first named them, with their Excel
    /// codes.
    formats: Vec<(DataStyle, String)>,
    /// Cell style name -> its data style, if any.
    cell_formats: HashMap<String, Option<usize>>,
}

/// A cell style as declared: its data style and the style it inherits.
struct CellStyle {
    data_style: Option<String>,
    parent: Option<String>,
}

/// Where a reading pass stands between pieces.
struct Pass {
    building: Option<Building>,
    /// Passing over a sheet not wanted, to its end.
    skipping: bool,
    /// Sheets seen, in order.
    names: Vec<String>,
    stopped: bool,
}

impl<'a> Spreadsheet<'a> {
    /// Opens the package and reads its styles: `styles.xml`, and
    /// `content.xml` up to its body.
    pub fn open(bytes: &'a [u8]) -> Result<Spreadsheet<'a>, OdsError> {
        let archive = ZipArchive::parse(bytes)?;
        if let Some(manifest) = part_text(&archive, "META-INF/manifest.xml")? {
            if manifest.contains("manifest:encryption-data") {
                return Err(OdsError::Encrypted);
            }
        }
        let mut data_styles = OdsStyles::default();
        let mut cell_styles: HashMap<String, CellStyle> = HashMap::new();
        let mut language = (String::new(), String::new());
        if let Some(styles) = part_text(&archive, "styles.xml")? {
            read_styles(&styles, &mut data_styles, &mut cell_styles, &mut language);
        }
        let mut spreadsheet = Spreadsheet {
            archive,
            formats: Vec::new(),
            cell_formats: HashMap::new(),
        };
        let (prelude, _, _) = spreadsheet.content_body()?;
        read_styles(&prelude, &mut data_styles, &mut cell_styles, &mut language);
        // Every cell style's data style, following parents.
        let mut by_data_style: HashMap<String, Option<usize>> = HashMap::new();
        for name in cell_styles.keys() {
            let mut current = Some(name.as_str());
            let mut found = None;
            let mut steps = 0;
            while let Some(style) = current.and_then(|name| cell_styles.get(name)) {
                if let Some(data) = &style.data_style {
                    found = Some(data.clone());
                    break;
                }
                current = style.parent.as_deref();
                steps += 1;
                if steps > 32 {
                    break;
                }
            }
            let index = found.and_then(|data| {
                if let Some(index) = by_data_style.get(&data) {
                    return *index;
                }
                let index = data_styles.resolve(&data).and_then(|mut style| {
                    style.localize(&language.0, &language.1);
                    if style.is_general() {
                        return None;
                    }
                    let code = numfmt::to_excel(&style);
                    spreadsheet.formats.push((style, code));
                    Some(spreadsheet.formats.len() - 1)
                });
                by_data_style.insert(data, index);
                index
            });
            spreadsheet.cell_formats.insert(name.clone(), index);
        }
        Ok(spreadsheet)
    }

    /// `content.xml` up to its body (the automatic styles), and a stream
    /// standing at the body with what it has read past it.
    fn content_body(&self) -> Result<(String, crate::io::zip::EntryStream<'a>, Vec<u8>), OdsError> {
        let entry = self
            .archive
            .find("content.xml")
            .ok_or(OdsError::Part("content.xml"))?;
        let mut stream = self.archive.stream(entry)?;
        let mut buffer = Vec::new();
        loop {
            let more = stream.next_piece(&mut buffer, PIECE)?;
            if let Some(at) = find(&buffer, b"<office:body") {
                let body = buffer.split_off(at);
                let prelude =
                    String::from_utf8(buffer).map_err(|_| OdsError::Part("content.xml"))?;
                return Ok((prelude, stream, body));
            }
            if !more {
                return Err(OdsError::Part("sheet"));
            }
        }
    }

    /// A data style and its Excel code.
    pub fn format(&self, index: usize) -> (&DataStyle, &str) {
        let (style, code) = &self.formats[index];
        (style, code)
    }

    /// Reads the sheets in order: each that `wanted` (by its index and
    /// name) takes is gathered and handed to `on_sheet`, which returns
    /// whether to go on. Returns the names of the sheets seen.
    pub fn read<E: From<OdsError>>(
        &self,
        need: Need,
        wanted: &mut dyn FnMut(usize, &str) -> bool,
        on_sheet: &mut dyn FnMut(Sheet) -> Result<bool, E>,
    ) -> Result<Vec<String>, E> {
        let (_, mut stream, mut buffer) = self.content_body()?;
        let mut pass = Pass {
            building: None,
            skipping: false,
            names: Vec::new(),
            stopped: false,
        };
        let mut more = true;
        let mut scratch = Scratch::new();
        while !pass.stopped {
            if more {
                more = stream
                    .next_piece(&mut buffer, PIECE)
                    .map_err(OdsError::from)?;
            }
            // Parse up to the last complete row (a cell's end inside a row
            // too long to wait for), all of it at the end.
            let end = if !more {
                buffer.len()
            } else if pass.skipping {
                buffer.len().saturating_sub(16)
            } else if let Some(at) = rfind(&buffer, b"</table:table-row>") {
                at + 18
            } else if buffer.len() > ROW_LIMIT {
                rfind(&buffer, b"</table:table-cell>").map_or(0, |at| at + 19)
            } else {
                0
            };
            // Cut at a character's start.
            let end = (0..=end)
                .rev()
                .find(|&at| at == buffer.len() || (buffer[at] as i8) >= -0x40)
                .unwrap_or(0);
            if end > 0 {
                let text = std::str::from_utf8(&buffer[..end])
                    .map_err(|_| OdsError::Part("content.xml"))?;
                self.parse(text, need, &mut pass, &mut scratch, wanted, on_sheet)?;
                buffer.drain(..end);
            }
            if !more {
                break;
            }
        }
        Ok(pass.names)
    }

    /// Reads one stretch of the body, picking up where the last left off.
    fn parse<E: From<OdsError>>(
        &self,
        text: &str,
        need: Need,
        pass: &mut Pass,
        cell: &mut Scratch,
        wanted: &mut dyn FnMut(usize, &str) -> bool,
        on_sheet: &mut dyn FnMut(Sheet) -> Result<bool, E>,
    ) -> Result<(), E> {
        let mut from = 0usize;
        loop {
            if pass.skipping {
                match find(&text.as_bytes()[from..], b"</table:table>") {
                    Some(at) => {
                        from += at + 14;
                        pass.skipping = false;
                    }
                    None => return Ok(()),
                }
            }
            let mut reader = XmlReader::new(&text[from..]);
            let mut resume = None;
            while let Some(event) = reader.next() {
                match event {
                    XmlEvent::Start {
                        name: "table:table",
                        attributes,
                        self_closing,
                    } => {
                        let index = pass.names.len();
                        let name = attribute(&attributes, "table:name")
                            .unwrap_or("Sheet")
                            .to_string();
                        pass.names.push(name.clone());
                        if !wanted(index, &name) {
                            if !self_closing {
                                pass.skipping = true;
                                resume = Some(from + reader.position());
                                break;
                            }
                            continue;
                        }
                        let building = Building::new(index, name, need);
                        if self_closing {
                            if !on_sheet(building.finish())? {
                                pass.stopped = true;
                                return Ok(());
                            }
                        } else {
                            pass.building = Some(building);
                        }
                    }
                    XmlEvent::End {
                        name: "table:table",
                    } => {
                        if let Some(building) = pass.building.take() {
                            if !on_sheet(building.finish())? {
                                pass.stopped = true;
                                return Ok(());
                            }
                        }
                    }
                    event => {
                        if let Some(building) = &mut pass.building {
                            self.sheet_event(building, event, &mut reader, cell);
                        } else if let XmlEvent::Start {
                            name,
                            self_closing: false,
                            ..
                        } = event
                        {
                            // Outside the sheets (named ranges, databases):
                            // nothing to read, but the containers.
                            if !matches!(name, "office:body" | "office:spreadsheet") {
                                skip(&mut reader, name);
                            }
                        }
                    }
                }
            }
            match resume {
                Some(at) => from = at,
                None => return Ok(()),
            }
        }
    }

    /// One event inside a sheet.
    fn sheet_event<'x>(
        &self,
        building: &mut Building,
        event: XmlEvent<'x>,
        reader: &mut XmlReader<'x>,
        cell: &mut Scratch,
    ) {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "table:table-column" => {
                    let repeat = number(&attributes, "table:number-columns-repeated");
                    let end = building.columns.last().map_or(0, |(end, _)| *end) + repeat;
                    building.columns.push((
                        end,
                        attribute(&attributes, "table:default-cell-style-name").map(str::to_string),
                    ));
                }
                "table:table-row" => {
                    building.row_repeat = number(&attributes, "table:number-rows-repeated");
                    building.row_style =
                        attribute(&attributes, "table:default-cell-style-name").map(str::to_string);
                    building.column = 0;
                    if self_closing {
                        building.end_row();
                    }
                }
                "table:table-cell" | "table:covered-table-cell" => {
                    let repeat = number(&attributes, "table:number-columns-repeated");
                    let spans = (
                        number(&attributes, "table:number-columns-spanned"),
                        number(&attributes, "table:number-rows-spanned"),
                    );
                    let own = attribute(&attributes, "table:style-name");
                    let column = building.column;
                    self.cell_value(&attributes, Some(style_at(own, building, column)), cell);
                    if !self_closing {
                        read_cell_text(reader, name, &mut cell.text);
                    }
                    let formula = attribute(&attributes, "table:formula").is_some();
                    self.finish_cell(cell, formula);
                    // A value repeated across columns whose default styles
                    // differ shows in each column's own format.
                    let varies = repeat > 1
                        && own.is_none()
                        && building.row_style.is_none()
                        && cell.has_content()
                        && matches!(
                            cell.value,
                            Value::Number(_) | Value::Date(_) | Value::Time(_)
                        )
                        && building
                            .columns
                            .iter()
                            .any(|(end, _)| *end > column && *end < column + repeat);
                    if varies {
                        let mut at = column;
                        while at < column + repeat {
                            let run_end = building
                                .columns
                                .iter()
                                .find(|(end, _)| *end > at)
                                .map_or(column + repeat, |(end, _)| (*end).min(column + repeat));
                            // Shown from the value under this column's
                            // format, not the text the file stored.
                            self.cell_value(&attributes, Some(style_at(own, building, at)), cell);
                            self.finish_cell(cell, formula);
                            building.push(cell, run_end - at, (1, 1));
                            at = run_end;
                        }
                    } else {
                        building.push(cell, repeat, spans);
                    }
                }
                // Rows inside these are the sheet's own.
                "table:table-header-rows"
                | "table:table-rows"
                | "table:table-row-group"
                | "table:table-header-columns"
                | "table:table-columns"
                | "table:table-column-group" => {}
                _ => {
                    if !self_closing {
                        skip(reader, name);
                    }
                }
            },
            XmlEvent::End {
                name: "table:table-row",
            } => building.end_row(),
            _ => {}
        }
    }

    /// A cell's value and format from its attributes.
    fn cell_value(
        &self,
        attributes: &[(&str, Cow<'_, str>)],
        style: Option<&str>,
        cell: &mut Scratch,
    ) {
        cell.text.clear();
        cell.raw.clear();
        cell.value = Value::Empty;
        cell.error = false;
        cell.format = style.and_then(|style| self.cell_formats.get(style).copied().flatten());
        let kind = attribute(attributes, "office:value-type");
        // An error shows LibreOffice's English name for it, which the
        // file may hold translated (`#WERT!`, `Chyba:502`).
        if attribute(attributes, "calcext:value-type") == Some("error") {
            cell.value = Value::Text;
            cell.error = true;
            return;
        }
        match kind {
            Some("float" | "percentage" | "currency") => {
                if let Some(text) = attribute(attributes, "office:value") {
                    let text = text.trim();
                    if let Ok(value) = text.parse::<f64>() {
                        cell.value = Value::Number(value);
                        cell.raw.push_str(text);
                    }
                }
            }
            Some("date") => {
                if let Some(text) = attribute(attributes, "office:date-value") {
                    if let Some(serial) = date_serial(text) {
                        cell.value = Value::Date(serial);
                        cell.raw.push_str(&iso_date(text));
                    }
                }
            }
            Some("time") => {
                if let Some(days) =
                    attribute(attributes, "office:time-value").and_then(duration_days)
                {
                    cell.value = Value::Time(days);
                    cell.raw.push_str(&iso_duration(days));
                }
            }
            Some("boolean") => {
                // `true`, or a number other than zero (`-1`).
                let value = attribute(attributes, "office:boolean-value").is_some_and(|text| {
                    text == "true" || text.trim().parse::<f64>().is_ok_and(|number| number != 0.0)
                });
                cell.value = Value::Boolean(value);
                cell.raw.push_str(if value { "TRUE" } else { "FALSE" });
            }
            Some("string") => {
                cell.value = Value::Text;
                if let Some(text) = attribute(attributes, "office:string-value") {
                    cell.raw.push_str(text);
                }
            }
            _ => {}
        }
    }

    /// The text when the cell has none of its own, and the raw form of a
    /// string.
    fn finish_cell(&self, cell: &mut Scratch, formula: bool) {
        if cell.value == Value::Empty && !cell.text.is_empty() {
            cell.value = Value::Text;
        }
        // A formula's error, in files that mark it only by its text.
        if formula && matches!(cell.value, Value::Number(_)) && is_error_text(&cell.text) {
            cell.value = Value::Text;
        }
        if cell.value == Value::Text {
            if cell.error {
                cell.text = english_error(&cell.text);
                cell.raw.clone_from(&cell.text);
                return;
            }
            if cell.text.is_empty() {
                cell.text.clone_from(&cell.raw);
            } else {
                cell.raw.clone_from(&cell.text);
            }
            // LibreOffice shows a one-line string through its text format.
            if let Some(shown) = cell
                .format
                .filter(|_| !cell.raw.contains('\n'))
                .and_then(|index| self.formats[index].0.render_text(&cell.raw))
            {
                cell.text = shown;
            }
            return;
        }
        // Numbers, dates, and booleans as LibreOffice shows them in
        // English: from the value and its format, unless the format names
        // a language of its own, which the file's text is already in.
        let own_language = cell
            .format
            .is_some_and(|index| self.formats[index].0.has_locale());
        if own_language && !cell.text.is_empty() {
            // A fill character, stored repeated (with zero-width spaces
            // between), shows once.
            if cell.text.contains('\u{200b}') {
                cell.text = collapse_fill(&cell.text);
            }
            return;
        }
        // General shows as many digits as the column fits: the file's text,
        // its decimal comma made a point.
        if let (Value::Number(value), None) = (cell.value, cell.format) {
            if let Some(text) = general_text(&cell.text, value) {
                cell.text = text;
                return;
            }
        }
        cell.text = match (cell.value, cell.format) {
            (Value::Boolean(_) | Value::Time(_), None) | (Value::Boolean(_), _) => cell.raw.clone(),
            (Value::Number(value) | Value::Date(value) | Value::Time(value), Some(index)) => {
                self.formats[index].0.render(value)
            }
            (Value::Number(value), None) => numfmt::general(value),
            (Value::Date(serial), None) => default_date(serial),
            _ => return,
        };
    }
}

/// `-\u{200b}-\u{200b}-` as `-`: each run of a character repeated with
/// zero-width spaces between, once.
fn collapse_fill(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut previous: Option<char> = None;
    let mut joined = false;
    for character in text.chars() {
        if character == '\u{200b}' {
            joined = true;
            continue;
        }
        if !(joined && previous == Some(character)) {
            out.push(character);
        }
        previous = Some(character);
        joined = false;
    }
    out
}

/// A General number's stored text in English, when it is the value shown
/// to fewer digits (`17,6` for 17.6028...).
fn general_text(text: &str, value: f64) -> Option<String> {
    let english = if text.contains(',') && !text.contains('.') {
        text.replacen(',', ".", 1)
    } else {
        text.to_string()
    };
    let shown: f64 = english.parse().ok()?;
    let close = (shown - value).abs() <= value.abs() * 1e-3 + 1e-9
        || (english.contains('.') && {
            let places = english.split('.').nth(1).map_or(0, |fraction| {
                fraction.split(['E', 'e']).next().map_or(0, str::len)
            });
            (shown - value).abs() <= 0.5 * 10f64.powi(-(places as i32)) + 1e-12
        });
    (close && english.len() <= 32).then_some(english)
}

/// An error's text in English: `Err:` codes and the error names in the
/// languages LibreOffice translates them to.
fn english_error(text: &str) -> String {
    if let Some((word, code)) = text.split_once(':') {
        if code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit()) && !word.contains(' ') {
            return format!("Err:{code}");
        }
    }
    const NAMES: [(&str, &[&str]); 7] = [
        (
            "#NULL!",
            &["#NUL!", "#NULO!", "#¡NULO!", "#LEEG!", "#NULLO!"],
        ),
        ("#DIV/0!", &["#¡DIV/0!", "#DEEL/0!"]),
        (
            "#VALUE!",
            &[
                "#WERT!",
                "#VALEUR!",
                "#¡VALOR!",
                "#VALOR!",
                "#WAARDE!",
                "#VALORE!",
                "#HODNOTA!",
            ],
        ),
        (
            "#REF!",
            &["#BEZUG!", "#¡REF!", "#VERW!", "#RIF!", "#ODKAZ!"],
        ),
        (
            "#NAME?",
            &["#NOM?", "#¿NOMBRE?", "#NAAM?", "#NOME?", "#NÁZEV?"],
        ),
        (
            "#NUM!",
            &[
                "#ZAHL!",
                "#NOMBRE!",
                "#¡NUM!",
                "#GETAL!",
                "#NÚM!",
                "#ČÍSLO!",
            ],
        ),
        ("#N/A", &["#NV", "#N/D", "#N/B", "#NENÍ_K_DISPOZICI"]),
    ];
    for (english, translations) in NAMES {
        if text == english || translations.contains(&text) {
            return english.to_string();
        }
    }
    text.to_string()
}

/// An error value's text: `#DIV/0!`, `#N/A`, `Err:502`, and their
/// translations (`#VALORE!`, `#NV`).
fn is_error_text(text: &str) -> bool {
    if let Some(code) = text.strip_prefix("Err:") {
        return code.len() == 3 && code.bytes().all(|b| b.is_ascii_digit());
    }
    let Some(rest) = text.strip_prefix('#') else {
        return false;
    };
    let length = rest.chars().count();
    (2..=12).contains(&length)
        && !rest.contains(char::is_whitespace)
        && (rest.ends_with(['!', '?']) || matches!(rest, "N/A" | "NV" | "N/D" | "NA"))
}

/// A date without a format, as LibreOffice shows one in English.
fn default_date(serial: f64) -> String {
    let code = if serial.fract() == 0.0 {
        "MM/DD/YY"
    } else {
        "MM/DD/YY HH:MM"
    };
    numfmt::from_excel(code).map_or_else(String::new, |style| style.render(serial))
}

/// A cell's style: its own, else its row's default, else its column's.
fn style_at<'b>(own: Option<&'b str>, building: &'b Building, at: usize) -> &'b str {
    own.or(building.row_style.as_deref())
        .or_else(|| {
            building
                .columns
                .iter()
                .find(|(end, _)| *end > at)
                .and_then(|(_, style)| style.as_deref())
        })
        .unwrap_or("Default")
}

/// The first and last places of `needle` in `haystack`.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

fn rfind(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .rposition(|window| window == needle)
}

/// Reads the data styles and cell styles of one part (`styles.xml`, or
/// `content.xml` before its body).
fn read_styles(
    xml: &str,
    data_styles: &mut OdsStyles,
    cell_styles: &mut HashMap<String, CellStyle>,
    document_language: &mut (String, String),
) {
    let mut reader = XmlReader::new(xml);
    while let Some(event) = reader.next() {
        let XmlEvent::Start {
            name,
            attributes,
            self_closing,
        } = event
        else {
            continue;
        };
        match name {
            _ if name.starts_with("number:") && name.ends_with("-style") => {
                data_styles.read_style(&mut reader, name, &attributes, self_closing);
            }
            "style:style" => {
                if attribute(&attributes, "style:family") == Some("table-cell") {
                    if let Some(style_name) = attribute(&attributes, "style:name") {
                        cell_styles.insert(
                            style_name.to_string(),
                            CellStyle {
                                data_style: attribute(&attributes, "style:data-style-name")
                                    .map(str::to_string),
                                parent: attribute(&attributes, "style:parent-style-name")
                                    .map(str::to_string),
                            },
                        );
                    }
                }
                if !self_closing {
                    skip(&mut reader, name);
                }
            }
            "style:default-style" => {
                if attribute(&attributes, "style:family") == Some("table-cell") && !self_closing {
                    // The document's language, for its built-in formats.
                    let mut inner = XmlReader::new(&xml[reader.position()..]);
                    for event in inner.by_ref() {
                        match event {
                            XmlEvent::Start {
                                name: "style:text-properties",
                                attributes,
                                ..
                            } => {
                                if let Some(language) = attribute(&attributes, "fo:language") {
                                    *document_language = (
                                        language.to_string(),
                                        attribute(&attributes, "fo:country")
                                            .unwrap_or("")
                                            .to_string(),
                                    );
                                }
                            }
                            XmlEvent::End {
                                name: "style:default-style",
                            } => break,
                            _ => {}
                        }
                    }
                }
                if attribute(&attributes, "style:family") == Some("table-cell") {
                    if let Some(data) = attribute(&attributes, "style:data-style-name") {
                        cell_styles.insert(
                            "Default".to_string(),
                            CellStyle {
                                data_style: Some(data.to_string()),
                                parent: None,
                            },
                        );
                    }
                }
                if !self_closing {
                    skip(&mut reader, name);
                }
            }
            _ => {}
        }
    }
}

/// The text a cell shows: its paragraphs joined by line breaks, its text
/// as written (LibreOffice keeps a cell's spaces), annotations and
/// drawings left out.
fn read_cell_text(reader: &mut XmlReader<'_>, cell: &str, out: &mut String) {
    let mut paragraphs = 0usize;
    let mut in_paragraph = 0usize;
    while let Some(event) = reader.next() {
        match event {
            XmlEvent::Start {
                name,
                attributes,
                self_closing,
            } => match name {
                "text:p" | "text:h" => {
                    if paragraphs > 0 {
                        out.push('\n');
                    }
                    paragraphs += 1;
                    if !self_closing {
                        in_paragraph += 1;
                    }
                }
                "text:s" if in_paragraph > 0 => {
                    let count = attribute(&attributes, "text:c")
                        .and_then(|count| count.parse::<usize>().ok())
                        .unwrap_or(1)
                        .min(1024);
                    out.extend(std::iter::repeat_n(' ', count));
                    if !self_closing {
                        skip(reader, name);
                    }
                }
                "text:tab" if in_paragraph > 0 => {
                    out.push('\t');
                    if !self_closing {
                        skip(reader, name);
                    }
                }
                "text:line-break" if in_paragraph > 0 => {
                    out.push('\n');
                    if !self_closing {
                        skip(reader, name);
                    }
                }
                _ if name.starts_with("text:")
                    && (in_paragraph > 0
                        || matches!(name, "text:list" | "text:list-item" | "text:section")) =>
                {
                    // Spans, links, fields: their text is the cell's.
                    if matches!(name, "text:note" | "text:ruby-text") && !self_closing {
                        skip(reader, name);
                    }
                }
                _ => {
                    // Annotations, drawings, and anything else: not shown.
                    if !self_closing {
                        skip(reader, name);
                    }
                }
            },
            XmlEvent::Text(text) => {
                if in_paragraph > 0 {
                    out.push_str(&text);
                }
            }
            XmlEvent::End { name } => {
                if name == cell {
                    return;
                }
                if matches!(name, "text:p" | "text:h") {
                    in_paragraph = in_paragraph.saturating_sub(1);
                }
            }
        }
    }
}

/// Skips to the end of the element `name` just started.
fn skip(reader: &mut XmlReader<'_>, name: &str) {
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

fn attribute<'e>(attributes: &'e [(&str, Cow<'_, str>)], name: &str) -> Option<&'e str> {
    attributes
        .iter()
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.as_ref())
}

/// A repeat or span count, at least 1.
fn number(attributes: &[(&str, Cow<'_, str>)], name: &str) -> usize {
    attribute(attributes, name)
        .and_then(|text| text.trim().parse::<usize>().ok())
        .unwrap_or(1)
        .max(1)
}

fn part_text(zip: &ZipArchive<'_>, name: &'static str) -> Result<Option<String>, OdsError> {
    let Some(entry) = zip.find(name) else {
        return Ok(None);
    };
    let mut bytes = Vec::new();
    zip.read(entry, &mut bytes)?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|_| OdsError::Part(name))
}

/// `2024-01-05`, `2024-01-05T13:45:00.5` as days since 1899-12-30.
pub fn date_serial(text: &str) -> Option<f64> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let (date, time) = body.split_once('T').unwrap_or((body, ""));
    let mut fields = date.splitn(3, '-');
    let year: i64 = fields.next()?.parse().ok()?;
    let month: u32 = fields.next()?.parse().ok()?;
    let day: u32 = fields.next()?.get(..2)?.parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let year = if negative { -year } else { year };
    let days = days_from_civil(year, month, day) + 25_569;
    let mut seconds = 0.0;
    if !time.is_empty() {
        // Drop a zone (`Z`, `+02:00`).
        let time = time.trim_end_matches('Z');
        let time = time.split(['+']).next().unwrap_or(time);
        let mut parts = time.splitn(3, ':');
        let hours: f64 = parts.next()?.parse().ok()?;
        let minutes: f64 = parts.next().unwrap_or("0").parse().ok()?;
        let secs: f64 = parts.next().unwrap_or("0").parse().ok()?;
        seconds = hours * 3600.0 + minutes * 60.0 + secs;
    }
    Some(days as f64 + seconds / 86_400.0)
}

fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = i64::from(month);
    let day_of_year =
        (153 * (if month > 2 { month - 3 } else { month + 9 }) + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// An ODS date value in the xlsx reader's form: the date, and the time to
/// the second when it is not midnight.
fn iso_date(text: &str) -> String {
    let text = text.trim();
    let Some((date, time)) = text.split_once('T') else {
        return text.to_string();
    };
    let time = time.get(..8).unwrap_or(time);
    if time == "00:00:00" || time.is_empty() {
        date.to_string()
    } else {
        format!("{date}T{time}")
    }
}

/// `PT13H45M00S`, `-P1DT2H`, `PT0.5S` as days.
pub(crate) fn duration_days(text: &str) -> Option<f64> {
    let text = text.trim();
    let (negative, body) = match text.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, text),
    };
    let body = body.strip_prefix('P')?;
    let (date, time) = body.split_once('T').unwrap_or((body, ""));
    let mut seconds = 0.0;
    let mut number = String::new();
    for character in date.chars() {
        match character {
            'D' => {
                seconds += number.parse::<f64>().ok()? * 86_400.0;
                number.clear();
            }
            'Y' | 'M' | 'W' => return None,
            _ => number.push(character),
        }
    }
    for character in time.chars() {
        let unit = match character {
            'H' => 3600.0,
            'M' => 60.0,
            'S' => 1.0,
            _ => {
                number.push(character);
                continue;
            }
        };
        seconds += number.parse::<f64>().ok()? * unit;
        number.clear();
    }
    let days = seconds / 86_400.0;
    Some(if negative { -days } else { days })
}

/// A duration as `HH:MM:SS`, hours past a day kept (`36:00:00`).
fn iso_duration(days: f64) -> String {
    let total = (days.abs() * 86_400.0).round() as u64;
    let sign = if days < 0.0 && total > 0 { "-" } else { "" };
    format!(
        "{sign}{:02}:{:02}:{:02}",
        total / 3600,
        (total / 60) % 60,
        total % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_durations_become_serials() {
        assert_eq!(date_serial("2024-01-05"), Some(45_296.0));
        assert_eq!(date_serial("2024-01-05T12:00:00"), Some(45_296.5));
        assert_eq!(date_serial("1899-12-30"), Some(0.0));
        assert_eq!(duration_days("PT36H00M00S"), Some(1.5));
        assert_eq!(duration_days("-PT12H"), Some(-0.5));
        assert_eq!(duration_days("P1DT6H"), Some(1.25));
        assert_eq!(iso_date("2024-01-05T00:00:00"), "2024-01-05");
        assert_eq!(iso_date("2024-01-05T13:45:00.123"), "2024-01-05T13:45:00");
        assert_eq!(iso_duration(1.5), "36:00:00");
    }
}
