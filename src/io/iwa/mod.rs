//! iWork Archive (IWA) streams, the encoding inside `.pages`, `.numbers`,
//! and `.key` packages: Snappy chunks with Apple's own framing, holding a
//! protobuf stream of objects. Each object is an `ArchiveInfo` header (its
//! identifier and the type and length of each message that follows) and
//! then those messages.

pub mod stream;

pub use stream::{IwaError, IwaObject, MessageInfo, decompress_stream, parse_objects};
