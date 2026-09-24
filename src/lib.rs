//! Sublime: universal efficient file conversion.
//!
//! The library never prints. It emits typed events through a sink that the
//! caller provides. See `DOCS/CONTRIBUTING.md` for the drop-in recipe.

pub mod cli;
pub mod converter;
pub mod converters;
pub mod document;
pub mod event;
pub mod format;
#[cfg(feature = "dev-tools")]
pub mod inspect;
pub mod io;
pub mod planner;
pub mod registry;
