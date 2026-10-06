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

/// An input: a file, or an iWork document saved as a package folder
/// (`Budget.numbers/Index/...`), zipped in memory into the single-file
/// form the readers take.
pub(crate) enum InputSource {
    File(std::fs::File),
    Package(std::io::Cursor<Vec<u8>>),
}

impl std::io::Read for InputSource {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        match self {
            InputSource::File(file) => file.read(buffer),
            InputSource::Package(bytes) => bytes.read(buffer),
        }
    }
}

impl crate::converter::RewindableRead for InputSource {
    fn rewind(&mut self) -> std::io::Result<()> {
        match self {
            InputSource::File(file) => crate::converter::RewindableRead::rewind(file),
            InputSource::Package(bytes) => crate::converter::RewindableRead::rewind(bytes),
        }
    }

    fn seek_to(&mut self, position: u64) -> std::io::Result<()> {
        match self {
            InputSource::File(file) => file.seek_to(position),
            InputSource::Package(bytes) => bytes.seek_to(position),
        }
    }
}

/// Opens `path`: a package folder is zipped, anything else opened.
pub(crate) fn open_input(path: &std::path::Path) -> std::io::Result<InputSource> {
    if is_package_folder(path) {
        return zip_folder(path).map(|bytes| InputSource::Package(std::io::Cursor::new(bytes)));
    }
    std::fs::File::open(path).map(InputSource::File)
}

/// A Pages, Numbers, or Keynote document saved as a folder rather than a
/// file: one document, not a folder of inputs.
pub(crate) fn is_package_folder(path: &std::path::Path) -> bool {
    let iwork = path.extension().is_some_and(|extension| {
        ["pages", "numbers", "key"]
            .iter()
            .any(|known| extension.eq_ignore_ascii_case(known))
    });
    iwork && path.is_dir() && (path.join("Index").is_dir() || path.join("Index.zip").is_file())
}

/// The folder's files as a stored ZIP, named relative to it.
fn zip_folder(root: &std::path::Path) -> std::io::Result<Vec<u8>> {
    let mut files = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(&directory)? {
            let path = entry?.path();
            if path.is_dir() {
                pending.push(path);
            } else {
                files.push(path);
            }
        }
    }
    files.sort();
    let mut zip = crate::io::zip::ZipWriter::new(Vec::new());
    for path in files {
        let Ok(relative) = path.strip_prefix(root) else {
            continue;
        };
        let name: Vec<String> = relative
            .components()
            .map(|part| part.as_os_str().to_string_lossy().into_owned())
            .collect();
        zip.add(&name.join("/"), &std::fs::read(&path)?)?;
    }
    zip.finish()
}
