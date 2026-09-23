//! Renderers turn events into stderr text. Human or JSON lines.

pub mod human;
pub mod json_lines;

pub use human::HumanRenderer;
pub use json_lines::{JsonLinesRenderer, encode_event};
