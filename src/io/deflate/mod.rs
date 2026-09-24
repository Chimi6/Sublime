//! DEFLATE (RFC 1951) decompression. The compressor comes later, with the
//! first writer that needs it.

pub mod inflate;

pub use inflate::{InflateError, inflate};
