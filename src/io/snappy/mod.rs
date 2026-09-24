//! Snappy block decompression: one raw block (varint length, then literal
//! and copy tags). Stream and file framings are the callers' business;
//! iWork uses its own (see `io::iwa`).

pub mod decode;

pub use decode::{SnappyError, decompress_block, uncompressed_length};
