//! One module per command.

pub mod batch;
pub mod check;
pub mod convert;
pub mod formats;
pub mod inspect;
pub mod merge;
pub mod paths;

/// The `--font` file, read once for every conversion that sets text.
pub(crate) fn font_option(
    path: Option<&str>,
) -> Result<Option<std::sync::Arc<Vec<u8>>>, crate::cli::CliError> {
    let Some(path) = path else {
        return Ok(None);
    };
    std::fs::read(path)
        .map(|bytes| Some(std::sync::Arc::new(bytes)))
        .map_err(|error| crate::cli::CliError::Io {
            action: format!("reading the font {path}"),
            error,
        })
}
