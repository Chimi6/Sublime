//! N64 ROM byte orders. A cartridge is read by the console as big-endian
//! 32-bit words (`z64`); copiers and emulators also saved it with each
//! 16-bit pair swapped (`v64`, the Doctor V64) and with each word reversed
//! (`n64`). The bytes are otherwise the same, so changing order is a
//! permutation within each word, done here in place a buffer at a time.
//!
//! The header's first word is the cartridge's bus configuration, `0x80`
//! in its first byte on every cartridge (`0x80371240` on almost all), so
//! where that byte lies tells the order whatever the file is named.

use std::io::{self, Read, Write};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// `z64`: as the console reads it.
    Big,
    /// `v64`: each 16-bit pair swapped.
    Swapped,
    /// `n64`: each 32-bit word reversed.
    Little,
}

impl Order {
    pub fn name(&self) -> &'static str {
        match self {
            Order::Big => "big-endian (z64)",
            Order::Swapped => "byte-swapped (v64)",
            Order::Little => "little-endian (n64)",
        }
    }

    /// Where the header's first byte lands in a word of this order.
    fn first_byte_at(&self) -> usize {
        match self {
            Order::Big => 0,
            Order::Swapped => 1,
            Order::Little => 3,
        }
    }

    /// The order the header's first word is in: `expected` when it fits,
    /// else the one that does; `None` when no order puts `0x80` first.
    pub fn of_header(word: [u8; 4], expected: Order) -> Option<Order> {
        let fits = |order: &Order| word[order.first_byte_at()] == 0x80;
        if fits(&expected) {
            return Some(expected);
        }
        [Order::Big, Order::Swapped, Order::Little]
            .into_iter()
            .find(fits)
    }
}

/// Bytes moved per read and write: large enough that the calls cost
/// nothing beside the copy, small enough to stay in cache.
const BUFFER: usize = 1 << 18;

/// Rewrites whole words of `data` from `from` order to `to`. Each change
/// of order is its own inverse, so the pair names it in either direction.
pub fn reorder(data: &mut [u8], from: Order, to: Order) {
    match (from, to) {
        (Order::Big, Order::Swapped) | (Order::Swapped, Order::Big) => permute::<1, 0, 3, 2>(data),
        (Order::Big, Order::Little) | (Order::Little, Order::Big) => permute::<3, 2, 1, 0>(data),
        (Order::Swapped, Order::Little) | (Order::Little, Order::Swapped) => {
            permute::<2, 3, 0, 1>(data)
        }
        _ => {}
    }
}

/// Puts byte `A` of each word first, `B` second, and so on. Written as
/// a whole-array shuffle, which the compiler turns into vector shuffles
/// (a byte at a time through a `u32`, it does not).
fn permute<const A: usize, const B: usize, const C: usize, const D: usize>(data: &mut [u8]) {
    for chunk in data.chunks_exact_mut(4) {
        let word: &mut [u8; 4] = chunk.try_into().expect("chunks of four");
        *word = [word[A], word[B], word[C], word[D]];
    }
}

/// What `convert` found and did.
#[derive(Debug, PartialEq, Eq)]
pub struct Converted {
    /// The order the input was read in.
    pub from: Order,
    /// Whether the header said so (`false`: it was taken as named).
    pub recognised: bool,
    pub bytes: u64,
}

#[derive(Debug)]
pub enum N64Error {
    Io(io::Error),
    /// Fewer bytes than a header word.
    Short(u64),
    /// A length that is not whole words.
    Partial(u64),
}

impl From<io::Error> for N64Error {
    fn from(error: io::Error) -> Self {
        N64Error::Io(error)
    }
}

/// Copies a ROM from `input` to `output` in `to` order. The input's order
/// comes from its header, `named` being the one assumed when the header
/// fits it or fits none.
pub fn convert(
    input: &mut dyn Read,
    output: &mut dyn Write,
    named: Order,
    to: Order,
) -> Result<Converted, N64Error> {
    let mut buffer = vec![0u8; BUFFER];
    let mut filled = fill(input, &mut buffer)?;
    if filled < 4 {
        return Err(N64Error::Short(filled as u64));
    }
    let header = [buffer[0], buffer[1], buffer[2], buffer[3]];
    let found = Order::of_header(header, named);
    let from = found.unwrap_or(named);
    let mut bytes = 0u64;
    loop {
        let whole = filled - filled % 4;
        reorder(&mut buffer[..whole], from, to);
        output.write_all(&buffer[..whole])?;
        bytes += whole as u64;
        if whole < filled {
            return Err(N64Error::Partial(bytes + (filled - whole) as u64));
        }
        if filled < buffer.len() {
            break;
        }
        filled = fill(input, &mut buffer)?;
        if filled == 0 {
            break;
        }
    }
    Ok(Converted {
        from,
        recognised: found.is_some(),
        bytes,
    })
}

/// Reads until `buffer` is full or the input ends.
fn fill(input: &mut dyn Read, buffer: &mut [u8]) -> io::Result<usize> {
    let mut filled = 0;
    while filled < buffer.len() {
        match input.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;

    const BIG: [u8; 8] = [0x80, 0x37, 0x12, 0x40, 0x00, 0x00, 0x00, 0x0f];
    const SWAPPED: [u8; 8] = [0x37, 0x80, 0x40, 0x12, 0x00, 0x00, 0x0f, 0x00];
    const LITTLE: [u8; 8] = [0x40, 0x12, 0x37, 0x80, 0x0f, 0x00, 0x00, 0x00];

    fn of(order: Order) -> [u8; 8] {
        match order {
            Order::Big => BIG,
            Order::Swapped => SWAPPED,
            Order::Little => LITTLE,
        }
    }

    const ORDERS: [Order; 3] = [Order::Big, Order::Swapped, Order::Little];

    #[test]
    fn every_order_reaches_every_other() {
        for from in ORDERS {
            for to in ORDERS {
                let mut data = of(from);
                reorder(&mut data, from, to);
                assert_eq!(data, of(to), "{from:?} -> {to:?}");
            }
        }
    }

    #[test]
    fn the_header_tells_the_order() {
        for order in ORDERS {
            let word = [of(order)[0], of(order)[1], of(order)[2], of(order)[3]];
            for named in ORDERS {
                assert_eq!(Order::of_header(word, named), Some(order));
            }
        }
        // A cartridge with another bus configuration.
        assert_eq!(
            Order::of_header([0x27, 0x80, 0x40, 0x07], Order::Big),
            Some(Order::Swapped)
        );
        assert_eq!(Order::of_header(*b"PK\x03\x04", Order::Big), None);
        // Two bytes of 0x80: the name decides between the orders that fit.
        let both = [0x80, 0x80, 0x12, 0x40];
        assert_eq!(Order::of_header(both, Order::Swapped), Some(Order::Swapped));
        assert_eq!(Order::of_header(both, Order::Little), Some(Order::Big));
    }

    /// A reader handing out a few bytes at a time.
    struct Trickle<'a>(&'a [u8]);

    impl Read for Trickle<'_> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = self.0.len().min(buffer.len()).min(7);
            buffer[..count].copy_from_slice(&self.0[..count]);
            self.0 = &self.0[count..];
            Ok(count)
        }
    }

    #[test]
    fn converts_across_buffers_whatever_the_name() {
        let mut rom: Vec<u8> = (0..BUFFER * 2 + 12)
            .map(|index| (index * 7) as u8)
            .collect();
        rom[..4].copy_from_slice(&[0x80, 0x37, 0x12, 0x40]);
        let mut expected = rom.clone();
        reorder(&mut expected, Order::Big, Order::Little);
        let mut output = Vec::new();
        let done = convert(
            &mut Trickle(&rom),
            &mut output,
            Order::Swapped,
            Order::Little,
        )
        .unwrap();
        assert_eq!(output, expected);
        assert_eq!(
            done,
            Converted {
                from: Order::Big,
                recognised: true,
                bytes: rom.len() as u64
            }
        );
    }

    #[test]
    fn refuses_partial_words_and_short_files() {
        let mut output = Vec::new();
        let partial = convert(&mut &BIG[..6], &mut output, Order::Big, Order::Swapped);
        assert!(matches!(partial, Err(N64Error::Partial(6))));
        let short = convert(&mut &BIG[..3], &mut output, Order::Big, Order::Swapped);
        assert!(matches!(short, Err(N64Error::Short(3))));
    }
}
