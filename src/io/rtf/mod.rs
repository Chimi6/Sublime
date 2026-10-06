//! Rich Text Format: a tree of brace groups and control words over 8-bit
//! text. `lexer` splits the source into tokens, `reader` builds the
//! document model from them, and `writer` renders the model as RTF.

pub mod codepage;
pub mod lexer;
pub mod reader;
pub mod writer;

pub use reader::{RtfError, read_rtf};
pub use writer::write_rtf;
