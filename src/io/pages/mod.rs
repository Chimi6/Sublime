//! Apple Pages documents: a ZIP package of IWA streams (`io::iwa`) whose
//! objects are the Pages, shared text (TSWP), drawing (TSD), and table
//! (TST) archives. This module holds what is specific to Pages: the type
//! registry and, as they are mapped, the readers for its archives.

pub mod types;

pub use types::type_name;
