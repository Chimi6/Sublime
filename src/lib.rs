//! Sublime: universal efficient file conversion.
//!
//! The library never prints. It emits typed events through a sink that the
//! caller provides. See `DOCS/CONTRIBUTING.md` for the drop-in recipe.

pub mod converter;
pub mod converters;
pub mod event;
pub mod format;
pub mod io;
pub mod registry;
