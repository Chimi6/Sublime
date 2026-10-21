//! N64 ROMs between their three byte orders, `z64`, `v64`, and `n64`
//! (`io::n64`). Every byte is kept; only its place within each 32-bit word
//! moves. The input's order is read from its header, so a ROM named for
//! one order and saved in another still comes out right, and one already
//! in the target order is copied unchanged.

use std::io::Write;

use crate::converter::{ConvertError, Converter, Entry, Fidelity, Input, Location, Tier};
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::n64::{self, N64Error, Order};

pub struct Reorder {
    name: &'static str,
    from: (&'static Format, Order),
    to: (&'static Format, Order),
}

const Z64: (&Format, Order) = (&formats::Z64, Order::Big);
const V64: (&Format, Order) = (&formats::V64, Order::Swapped);
const N64: (&Format, Order) = (&formats::N64, Order::Little);

pub static Z64_TO_V64: Reorder = Reorder {
    name: "z64-to-v64",
    from: Z64,
    to: V64,
};

pub static Z64_TO_N64: Reorder = Reorder {
    name: "z64-to-n64",
    from: Z64,
    to: N64,
};

pub static V64_TO_Z64: Reorder = Reorder {
    name: "v64-to-z64",
    from: V64,
    to: Z64,
};

pub static V64_TO_N64: Reorder = Reorder {
    name: "v64-to-n64",
    from: V64,
    to: N64,
};

pub static N64_TO_Z64: Reorder = Reorder {
    name: "n64-to-z64",
    from: N64,
    to: Z64,
};

pub static N64_TO_V64: Reorder = Reorder {
    name: "n64-to-v64",
    from: N64,
    to: V64,
};

impl Converter for Reorder {
    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let (named, to) = (self.from.1, self.to.1);
        let done = n64::convert(&mut input, output, named, to).map_err(|error| match error {
            N64Error::Io(error) => ConvertError::Io(error),
            N64Error::Short(bytes) => {
                malformed(format!("{bytes} bytes is too short to be an N64 ROM"))
            }
            N64Error::Partial(bytes) => malformed(format!(
                "an N64 ROM is whole 32-bit words, and this one is {bytes} bytes"
            )),
        })?;
        if !done.recognised {
            context.warning(format!(
                "the first bytes are not an N64 ROM header; read as {}, as named",
                named.name()
            ));
        } else if done.from != named {
            context.warning(format!(
                "the ROM is {}, not {} as named; read as {}",
                done.from.name(),
                named.name(),
                done.from.name()
            ));
        }
        if done.from == to {
            context.warning(format!(
                "the ROM is already {}; copied unchanged",
                to.name()
            ));
        }
        Ok(())
    }
}

impl Reorder {
    pub const fn entry(&'static self) -> Entry {
        Entry {
            name: self.name,
            from: self.from.0,
            to: self.to.0,
            fidelity: Fidelity::Lossless,
            tier: Tier::Native,
            converter: self,
        }
    }
}

fn malformed(message: String) -> ConvertError {
    ConvertError::Malformed {
        location: Location::default(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declares_six_lossless_edges() {
        let all = [
            &Z64_TO_V64,
            &Z64_TO_N64,
            &V64_TO_Z64,
            &V64_TO_N64,
            &N64_TO_Z64,
            &N64_TO_V64,
        ];
        for converter in all {
            let expected = format!(
                "{}-to-{}",
                converter.entry().from.id,
                converter.entry().to.id
            );
            assert_eq!(converter.entry().name, expected);
            assert_eq!(converter.entry().fidelity, Fidelity::Lossless);
        }
    }
}
