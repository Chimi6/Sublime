//! Markdown, HTML, text, Word, Pages, and RTF -> PDF. Each reader's Markdown
//! event stream is set as pages by the composer (`io::pdf::compose`):
//! Letter pages, one-inch margins, the base-14 Helvetica and Courier.

use std::io::{Read, Write};
use std::sync::Arc;

use crate::converter::{ConvertError, Converter, Fidelity, Input, Location, Tier};
use crate::converters::input::read_text_document;
use crate::converters::pages_to_json::package_error;
use crate::document::Document;
use crate::document::markdown::emit_events;
use crate::event::Context;
use crate::format::Format;
use crate::format::formats;
use crate::io::docx::read_docx;
use crate::io::font::Font;
use crate::io::markdown::{EventSink, Options};
use crate::io::pages::{Package, Scope, read_document};
use crate::io::pdf::PdfDocument;
use crate::io::pdf::compose::{ComposeNotes, Composer, Faces, ImageSource, PageSetup, Picture};
use crate::io::png::RowSink;

const FIDELITY_NOTE: &str = "set on Letter pages in Helvetica and Courier, with fonts installed on this machine (subset and embedded) for other scripts; images set as blocks at their size, scaled to the page (JPEG embedded unchanged), and shown by their alt text when not found (web addresses are not fetched), not readable (GIF), or in a table; raw HTML is dropped, and characters no installed font covers become ?";

#[derive(Clone, Copy)]
enum Source {
    Markdown,
    Html,
    Text,
    Docx,
    Pages,
    Rtf,
}

pub struct ToPdf {
    name: &'static str,
    from: &'static Format,
    source: Source,
}

pub static MARKDOWN_TO_PDF: ToPdf = ToPdf {
    name: "markdown-to-pdf",
    from: &formats::MARKDOWN,
    source: Source::Markdown,
};

pub static HTML_TO_PDF: ToPdf = ToPdf {
    name: "html-to-pdf",
    from: &formats::HTML,
    source: Source::Html,
};

pub static TEXT_TO_PDF: ToPdf = ToPdf {
    name: "text-to-pdf",
    from: &formats::TEXT,
    source: Source::Text,
};

pub static DOCX_TO_PDF: ToPdf = ToPdf {
    name: "docx-to-pdf",
    from: &formats::DOCX,
    source: Source::Docx,
};

pub static PAGES_TO_PDF: ToPdf = ToPdf {
    name: "pages-to-pdf",
    from: &formats::PAGES,
    source: Source::Pages,
};

pub static RTF_TO_PDF: ToPdf = ToPdf {
    name: "rtf-to-pdf",
    from: &formats::RTF,
    source: Source::Rtf,
};

impl Converter for ToPdf {
    fn name(&self) -> &'static str {
        self.name
    }

    fn from(&self) -> &'static Format {
        self.from
    }

    fn to(&self) -> &'static Format {
        &formats::PDF
    }

    fn fidelity(&self) -> Fidelity {
        Fidelity::Conditional(FIDELITY_NOTE)
    }

    fn tier(&self) -> Tier {
        Tier::Native
    }

    fn convert(
        &self,
        mut input: Input<'_>,
        output: &mut dyn Write,
        context: &mut Context<'_>,
    ) -> Result<(), ConvertError> {
        let forced = match &context.options.font {
            Some(bytes) => Some(Arc::new(Font::parse(bytes.clone(), 0).map_err(
                |error| ConvertError::Malformed {
                    location: Location::default(),
                    message: format!("the font: {error}"),
                },
            )?)),
            None => None,
        };
        if forced.as_ref().is_some_and(|font| !font.has_glyf()) {
            return Err(ConvertError::Malformed {
                location: Location::default(),
                message: "the font has CFF outlines; only TrueType-outline fonts (.ttf) are embedded so far".to_string(),
            });
        }
        let faces = Faces::new(forced, true);
        let mut document = PdfDocument::new(&mut *output)?;
        let notes = match self.source {
            Source::Markdown | Source::Html | Source::Text => {
                let text = read_text_document(&mut input)?;
                let mut images = FileImages::new(context.options.base.clone());
                let mut composer = Composer::new(&mut document, PageSetup::default(), faces)
                    .with_images(&mut images);
                let sink = &mut composer as &mut dyn EventSink<'_>;
                match self.source {
                    Source::Markdown => {
                        crate::io::markdown::parse_into(&text, Options::default(), sink)
                    }
                    Source::Html => crate::io::html::reader::parse_into(&text, sink),
                    _ => crate::io::text::reader::parse_into(&text, sink),
                }
                let notes = composer.finish()?;
                images.report(context);
                notes
            }
            Source::Docx | Source::Pages | Source::Rtf => {
                let model = match self.source {
                    Source::Docx => {
                        let mut bytes = Vec::new();
                        input.read_to_end(&mut bytes)?;
                        read_docx(&bytes).map_err(|error| ConvertError::Malformed {
                            location: Location::default(),
                            message: error.to_string(),
                        })?
                    }
                    Source::Pages => {
                        let mut bytes = Vec::new();
                        input.read_to_end(&mut bytes)?;
                        let package =
                            Package::read_scope(&bytes, Scope::Document).map_err(package_error)?;
                        read_document(&package)
                    }
                    _ => crate::converters::rtf::read_input(&mut input)?,
                };
                let mut images = ModelImages::new(&model);
                let mut composer = Composer::new(&mut document, PageSetup::default(), faces)
                    .with_images(&mut images);
                emit_events(&model, &mut composer as &mut dyn EventSink<'_>);
                composer.finish()?
            }
        };
        report(&notes, self.name, context);
        document.finish()?;
        output.flush()?;
        Ok(())
    }
}

fn report(notes: &ComposeNotes, name: &'static str, context: &mut Context<'_>) {
    if notes.unset_characters > 0 {
        context.loss(
            name,
            Location::default(),
            format!(
                "{} characters no available font covers set as ?",
                notes.unset_characters
            ),
        );
    }
    if notes.images > 0 {
        context.warning(format!(
            "{} images shown by their alt text (not found, not readable, or in a table)",
            notes.images
        ));
    }
    if notes.html > 0 {
        context.warning(format!("{} pieces of raw HTML dropped", notes.html));
    }
}

/// The largest picture read from a path for a PDF.
const MAX_IMAGE_BYTES: u64 = 64 << 20;

/// A Markdown, HTML, or text document's images: a `data:` URI's bytes, or
/// a file by its path from the input's folder (on the command line; the
/// WebAssembly module has no files). Web addresses are not fetched.
struct FileImages {
    base: Option<std::path::PathBuf>,
    remote: usize,
}

impl FileImages {
    fn new(base: Option<std::path::PathBuf>) -> FileImages {
        FileImages { base, remote: 0 }
    }

    fn report(&self, context: &mut Context<'_>) {
        if self.remote > 0 {
            context.warning(format!(
                "{} images by web address not fetched (shown by their alt text)",
                self.remote
            ));
        }
    }
}

impl ImageSource for FileImages {
    fn resolve(&mut self, destination: &str) -> Option<Picture> {
        let destination = destination.trim();
        if let Some(data) = destination.strip_prefix("data:") {
            let (header, payload) = data.split_once(',')?;
            if !header.ends_with(";base64") {
                return None;
            }
            let mut bytes = Vec::new();
            crate::io::base64::decode(payload.trim(), &mut bytes)?;
            return Some((bytes, None));
        }
        let lower = destination.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("//")
        {
            self.remote += 1;
            return None;
        }
        let path = percent_decoded(destination.strip_prefix("file://").unwrap_or(destination));
        let path = std::path::Path::new(&path);
        let path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base.as_ref()?.join(path)
        };
        let file = std::fs::File::open(path).ok()?;
        let mut bytes = Vec::new();
        file.take(MAX_IMAGE_BYTES).read_to_end(&mut bytes).ok()?;
        Some((bytes, None))
    }

    fn decode(&mut self, bytes: &[u8], sink: &mut dyn RowSink) -> std::io::Result<()> {
        crate::converters::image::decode_image(bytes, sink)
    }
}

/// A path's `%XX` escapes decoded (`My%20Photo.png`).
fn percent_decoded(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && let Some(value) = text
                .get(index + 1..index + 3)
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(value);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// A document model's images (Word, Pages, RTF): its media by file name,
/// each showing at the size the document gives it there. A picture shown
/// several times takes its sizes in document order, the last for any
/// showing past them (a floating copy).
struct ModelImages<'d> {
    document: &'d Document,
    sizes: std::collections::HashMap<usize, (Vec<(f64, f64)>, usize)>,
}

impl<'d> ModelImages<'d> {
    fn new(document: &'d Document) -> ModelImages<'d> {
        let mut sizes: std::collections::HashMap<usize, (Vec<(f64, f64)>, usize)> =
            std::collections::HashMap::new();
        for image in &document.images {
            if image.width > 0.0 && image.height > 0.0 {
                sizes
                    .entry(image.media)
                    .or_default()
                    .0
                    .push((f64::from(image.width), f64::from(image.height)));
            }
        }
        ModelImages { document, sizes }
    }
}

impl ImageSource for ModelImages<'_> {
    fn resolve(&mut self, destination: &str) -> Option<Picture> {
        let (index, media) = self
            .document
            .media
            .iter()
            .enumerate()
            .find(|(_, media)| media.name == destination)?;
        let size = self.sizes.get_mut(&index).and_then(|(sizes, shown)| {
            let size = sizes.get(*shown).or(sizes.last()).copied();
            *shown += 1;
            size
        });
        Some((media.bytes.clone(), size))
    }

    fn decode(&mut self, bytes: &[u8], sink: &mut dyn RowSink) -> std::io::Result<()> {
        crate::converters::image::decode_image(bytes, sink)
    }
}
