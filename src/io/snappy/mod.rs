//! Snappy block decompression: one raw block (varint length, then literal
//! and copy tags). Stream and file framings are the callers' business;
//! iWork uses its own (see `io::iwa`).

pub mod decode;
pub mod encode;

pub use decode::{SnappyError, decompress_block, uncompressed_length};
pub use encode::encode_literal_block;
