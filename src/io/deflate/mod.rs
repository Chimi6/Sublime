//! DEFLATE (RFC 1951) in both directions.

pub mod compress;
pub mod inflate;

pub use compress::{Level, deflate, deflate_part};
pub use inflate::{InflateError, Inflater, Progress, inflate};
