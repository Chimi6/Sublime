//! DEFLATE (RFC 1951) in both directions.

pub mod compress;
pub mod inflate;

pub use compress::deflate;
pub use inflate::{InflateError, inflate};
