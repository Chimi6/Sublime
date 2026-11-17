//! Limits on what a hostile file can make a conversion do: how deep it
//! nests, how much it expands, and how large a count or picture it may
//! declare. Every reader takes its guards from here, so one constant
//! decides each limit and every refusal reads the same.
//!
//! The guards hold no global state. A reader keeps a `Depth` or a
//! `Budget` in its own struct, which lives as long as one conversion.

use std::fmt;
use std::io;

/// The deepest nesting a reader follows: arrays in arrays, tables in
/// table cells, block quotes in block quotes. The browser build's stack
/// held about 2,000 levels of the worst parser measured, so this keeps a
/// wide margin; no real document nests anywhere near it.
pub const NESTING: u32 = 256;

/// The stack a conversion runs on (its own thread, and each hop's and
/// batch worker's): what Linux and macOS give the main thread. Windows
/// gives 1 MiB, too little for `NESTING` levels of the debug build.
pub const STACK: usize = 8 << 20;

/// The most bytes a conversion may decode, expand or hold for one input:
/// the hub's guard against a header or a compressed stream that asks for
/// more memory than any real file needs (2 GiB).
pub const BUDGET: u64 = 1 << 31;

/// The longest side of a picture: a row is allocated whole, often twice,
/// before any pixel arrives, so a side is capped beside the total.
/// libpng's default is under a million; no camera or scanner comes near.
pub const SIDE: u32 = 1 << 20;

/// The empty cells a reader may invent to fill out one document's tables
/// (short rows padded, spans covered): cmark-gfm's cap on table
/// autocompletion, so a ragged table cannot grow quadratically.
pub const FILLER_CELLS: u64 = 0x80000;

/// Why a guard refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LimitError {
    /// Nesting passed `NESTING`.
    TooDeep,
    /// Decoded or expanded data passed its budget.
    OverBudget,
    /// A count, size or picture from the file is too large to hold.
    TooLarge,
}

impl LimitError {
    pub fn message(self) -> &'static str {
        match self {
            LimitError::TooDeep => "nested more than 256 levels deep",
            LimitError::OverBudget => "expands past the 2 GiB limit",
            LimitError::TooLarge => "declares a size too large to hold",
        }
    }
}

impl fmt::Display for LimitError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.message())
    }
}

impl std::error::Error for LimitError {}

impl From<LimitError> for io::Error {
    fn from(error: LimitError) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, error)
    }
}

/// A nesting counter: `enter` where a container opens, `leave` where it
/// closes. A reader that skips what lies past the limit, instead of
/// failing, must still pair every successful `enter` with a `leave`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Depth(u32);

impl Depth {
    pub fn enter(&mut self) -> Result<(), LimitError> {
        if self.0 >= NESTING {
            return Err(LimitError::TooDeep);
        }
        self.0 += 1;
        Ok(())
    }

    pub fn leave(&mut self) {
        self.0 = self.0.saturating_sub(1);
    }

    pub fn level(&self) -> u32 {
        self.0
    }
}

/// What is left to spend of a cap on decoded or produced units (bytes,
/// unless the owner says otherwise).
#[derive(Debug, Clone, Copy)]
pub struct Budget(u64);

impl Default for Budget {
    fn default() -> Budget {
        Budget(BUDGET)
    }
}

impl Budget {
    pub fn new(units: u64) -> Budget {
        Budget(units)
    }

    /// Spends `units`, or fails and spends nothing.
    pub fn take(&mut self, units: u64) -> Result<(), LimitError> {
        self.0 = self.0.checked_sub(units).ok_or(LimitError::OverBudget)?;
        Ok(())
    }

    pub fn left(&self) -> u64 {
        self.0
    }
}

/// A count read from a file, cut to `max` (counts past what the format
/// allows are damage, not intent).
pub fn clamp(count: u64, max: usize) -> usize {
    usize::try_from(count).map_or(max, |count| count.min(max))
}

/// A count written in a file as decimal text, cut to `max`; `None` when
/// the text is not a count.
pub fn count(text: &str, max: usize) -> Option<usize> {
    text.trim()
        .parse::<u64>()
        .ok()
        .map(|count| clamp(count, max))
}

/// `count` items of `each` bytes as one allocation's length, when that
/// fits the budget and this machine's `usize` (32 bits in the browser).
pub fn bytes(count: u64, each: u64) -> Result<usize, LimitError> {
    count
        .checked_mul(each)
        .filter(|total| *total <= BUDGET)
        .and_then(|total| usize::try_from(total).ok())
        .ok_or(LimitError::TooLarge)
}

/// A picture's size, checked once so no reader multiplies by hand: no
/// side is zero or past `SIDE`, a row and the whole image fit the budget
/// and `usize`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Dimensions {
    pub width: u32,
    pub height: u32,
    row: usize,
    total: usize,
}

impl Dimensions {
    /// A picture of `bits` bits per pixel (all channels and samples); a
    /// row is padded to whole bytes.
    pub fn new(width: u32, height: u32, bits: u64) -> Result<Dimensions, LimitError> {
        if width == 0 || height == 0 || bits == 0 || width.max(height) > SIDE {
            return Err(LimitError::TooLarge);
        }
        let row = u64::from(width)
            .checked_mul(bits)
            .map(|bits| bits.div_ceil(8))
            .ok_or(LimitError::TooLarge)?;
        Ok(Dimensions {
            width,
            height,
            row: bytes(row, 1)?,
            total: bytes(row, u64::from(height))?,
        })
    }

    /// The bytes of one row.
    pub fn row_bytes(&self) -> usize {
        self.row
    }

    /// The bytes of every row.
    pub fn total_bytes(&self) -> usize {
        self.total
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_refuses_past_the_limit_and_recovers_on_leave() {
        let mut depth = Depth::default();
        for _ in 0..NESTING {
            depth.enter().unwrap();
        }
        assert_eq!(depth.enter(), Err(LimitError::TooDeep));
        depth.leave();
        assert!(depth.enter().is_ok());
        assert_eq!(depth.level(), NESTING);
    }

    #[test]
    fn budget_spends_until_empty() {
        let mut budget = Budget::new(10);
        budget.take(6).unwrap();
        assert_eq!(budget.take(5), Err(LimitError::OverBudget));
        assert_eq!(budget.left(), 4);
        budget.take(4).unwrap();
        assert_eq!(Budget::default().left(), BUDGET);
    }

    #[test]
    fn counts_clamp_and_sizes_check() {
        assert_eq!(clamp(5, 3), 3);
        assert_eq!(clamp(u64::MAX, 63), 63);
        assert_eq!(clamp(2, 3), 2);
        assert_eq!(count(" 4294967295", 63), Some(63));
        assert_eq!(count("7", 63), Some(7));
        assert_eq!(count("-1", 63), None);
        assert_eq!(bytes(1 << 20, 8), Ok(8 << 20));
        assert_eq!(bytes(u64::MAX, 2), Err(LimitError::TooLarge));
        assert_eq!(bytes(BUDGET, 2), Err(LimitError::TooLarge));
    }

    #[test]
    fn dimensions_count_real_sample_bytes() {
        let rgba16 = Dimensions::new(100, 10, 64).unwrap();
        assert_eq!(rgba16.row_bytes(), 800);
        assert_eq!(rgba16.total_bytes(), 8000);
        let bilevel = Dimensions::new(9, 2, 1).unwrap();
        assert_eq!(bilevel.row_bytes(), 2);
        // 69 bytes of PNG declared 2^29 by 1 at 16-bit RGBA: 4 GiB a row.
        assert!(Dimensions::new(1 << 29, 1, 64).is_err());
        assert!(Dimensions::new(0, 5, 8).is_err());
        assert!(Dimensions::new(u32::MAX, u32::MAX, 32).is_err());
        // 16 bytes of PBM declared 1 by 2,000,000,000.
        assert!(Dimensions::new(1, 2_000_000_000, 1).is_err());
    }
}
