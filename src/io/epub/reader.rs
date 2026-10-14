//! Reading an EPUB into the document model: `META-INF/container.xml`
//! names the package document (the OPF), whose manifest lists the files
//! and whose spine orders the chapters. Each chapter is XHTML, read by the
//! HTML reader into one document: a chapter starts a new page, its
//! pictures come from the package, its notes (in it or in a notes file of
//! their own) become footnotes, and links between the book's files, which
//! no other format can follow, keep their text. The navigation document
//! is the reader's table of contents, not text, and is left out, as are
//! chapters the spine marks as not in reading order (`linear="no"`).

use std::borrow::Cow;
use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use super::is_external;
use crate::document::Document;
use crate::document::from_events::DocumentBuilder;
use crate::io::markdown::{Event, EventSink, Tag, TagEnd};
use crate::io::scan::percent_decoded;
use crate::io::xml::{XmlEvent, XmlReader};
use crate::io::zip::{ZipArchive, ZipError};

#[derive(Debug)]
pub enum EpubError {
    Zip(ZipError),
    /// A file the book needs is missing or not UTF-8.
    Part(String),
}

impl fmt::Display for EpubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EpubError::Zip(error) => write!(formatter, "{error}"),
            EpubError::Part(name) => write!(formatter, "the book has no readable {name}"),
        }
    }
}

impl std::error::Error for EpubError {}

impl From<ZipError> for EpubError {
    fn from(error: ZipError) -> Self {
        EpubError::Zip(error)
    }
}

/// A manifest entry: its path in the package and what it is.
struct Item {
    id: String,
    path: String,
    media_type: String,
    properties: String,
}

/// The book's chapters in reading order, as package paths.
pub(super) struct Package {
    pub chapters: Vec<String>,
    /// The cover picture's package path, when the book names one.
    pub cover: Option<String>,
}

pub fn read_epub(bytes: &[u8]) -> Result<Document, EpubError> {
    let zip = ZipArchive::parse(bytes)?;
    let package = read_package(&zip)?;
    // The chapter being read, which an image's relative source starts from.
    let directory = Rc::new(RefCell::new(String::new()));
    let mut builder = DocumentBuilder::new();
    let images_directory = Rc::clone(&directory);
    let images = &zip;
    builder.resolve_images(Box::new(move |source: &str| {
        let path = resolve(&images_directory.borrow(), source)?;
        let entry = images.find(&path)?;
        let mut bytes = Vec::new();
        images.read(entry, &mut bytes).ok()?;
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        Some((name, bytes))
    }));
    let mut texts: Vec<(&str, Vec<u8>)> = Vec::with_capacity(package.chapters.len());
    for chapter in &package.chapters {
        let Some(entry) = zip.find(chapter) else {
            continue;
        };
        let mut bytes = Vec::new();
        zip.read(entry, &mut bytes)?;
        texts.push((chapter, bytes));
    }
    // The cover comes first, unless a page of the book shows it already.
    let mut first = true;
    if let Some(cover) = &package.cover {
        // By its file name, as written or with its spaces escaped.
        let name = cover.rsplit('/').next().unwrap_or(cover);
        let escaped = name.replace(' ', "%20");
        let shown = texts.iter().any(|(_, bytes)| {
            [name, escaped.as_str()].iter().any(|name| {
                bytes
                    .windows(name.len())
                    .any(|window| window == name.as_bytes())
            })
        });
        if !shown {
            let mut sink = BookLinks {
                builder: &mut builder,
                kept: Vec::new(),
            };
            sink.event(Event::Start(Tag::Paragraph));
            sink.event(Event::Start(Tag::Image {
                destination: Cow::Owned(format!("/{cover}")),
                title: Cow::Borrowed(""),
            }));
            sink.event(Event::End(TagEnd::Image));
            sink.event(Event::End(TagEnd::Paragraph));
            first = false;
        }
    }
    for (chapter, bytes) in &texts {
        let text = String::from_utf8_lossy(bytes);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        *directory.borrow_mut() = parent(chapter).to_string();
        if !first {
            builder.break_page();
        }
        first = false;
        let mut sink = BookLinks {
            builder: &mut builder,
            kept: Vec::new(),
        };
        crate::io::html::reader::parse_into(text, &mut sink);
    }
    Ok(builder.finish())
}

/// The package document's manifest and spine.
pub(super) fn read_package(zip: &ZipArchive<'_>) -> Result<Package, EpubError> {
    let container = part_text(zip, "META-INF/container.xml");
    let opf_path = container
        .as_deref()
        .and_then(|text| {
            XmlReader::new(text).find_map(|event| match event {
                XmlEvent::Start {
                    name, attributes, ..
                } if local(name) == "rootfile" => attributes
                    .into_iter()
                    .find(|(key, _)| *key == "full-path")
                    .map(|(_, value)| value.into_owned()),
                _ => None,
            })
        })
        // A book without a container: its first package document.
        .or_else(|| {
            zip.entries()
                .iter()
                .find(|entry| entry.name.ends_with(".opf"))
                .map(|entry| entry.name.clone())
        })
        .ok_or_else(|| EpubError::Part("package document".to_string()))?;
    let opf = part_text(zip, &opf_path).ok_or_else(|| EpubError::Part(opf_path.clone()))?;
    let base = parent(&opf_path).to_string();
    let mut items: Vec<Item> = Vec::new();
    // EPUB 2 names its cover with `<meta name="cover" content="id"/>`.
    let mut cover_id: Option<String> = None;
    let mut spine: Vec<String> = Vec::new();
    for event in XmlReader::new(&opf) {
        let XmlEvent::Start {
            name, attributes, ..
        } = event
        else {
            continue;
        };
        let get = |key: &str| {
            attributes
                .iter()
                .find(|(name, _)| *name == key)
                .map(|(_, value)| value.to_string())
        };
        match local(name) {
            "item" => {
                let href = get("href").unwrap_or_default();
                items.push(Item {
                    id: get("id").unwrap_or_default(),
                    path: resolve(&base, &href).unwrap_or(href),
                    media_type: get("media-type").unwrap_or_default(),
                    properties: get("properties").unwrap_or_default(),
                });
            }
            "meta" if get("name").as_deref() == Some("cover") => cover_id = get("content"),
            "itemref" if get("linear").as_deref() != Some("no") => {
                spine.push(get("idref").unwrap_or_default());
            }
            _ => {}
        }
    }
    let chapters = spine
        .iter()
        .filter_map(|id| items.iter().find(|item| item.id == *id))
        .filter(|item| {
            matches!(
                item.media_type.as_str(),
                "application/xhtml+xml" | "text/html" | "application/x-dtbook+xml"
            ) && !item
                .properties
                .split_ascii_whitespace()
                .any(|word| word == "nav")
        })
        .map(|item| item.path.clone())
        .collect();
    let cover = items
        .iter()
        .find(|item| {
            item.properties
                .split_ascii_whitespace()
                .any(|word| word == "cover-image")
        })
        .or_else(|| {
            let id = cover_id.as_deref()?;
            items.iter().find(|item| item.id == id)
        })
        .filter(|item| item.media_type.starts_with("image/"))
        .map(|item| item.path.clone());
    Ok(Package { chapters, cover })
}

fn part_text(zip: &ZipArchive<'_>, name: &str) -> Option<String> {
    let entry = zip.find(name)?;
    let mut bytes = Vec::new();
    zip.read(entry, &mut bytes).ok()?;
    String::from_utf8(bytes).ok()
}

/// An element's name without its prefix (`opf:item`, `item`).
fn local(name: &str) -> &str {
    name.rsplit(':').next().unwrap_or(name)
}

/// The directory part of a package path (`OEBPS/text`), empty at the root.
fn parent(path: &str) -> &str {
    path.rfind('/').map_or("", |slash| &path[..slash])
}

/// A reference relative to `directory`, as a package path: its fragment
/// and query dropped, escapes decoded, and `.` and `..` resolved. `None`
/// for a reference outside the package (a URL with a scheme).
pub(super) fn resolve(directory: &str, reference: &str) -> Option<String> {
    let reference = reference.split(['#', '?']).next().unwrap_or("");
    if reference.is_empty() || reference.contains("://") || reference.starts_with("data:") {
        return None;
    }
    let reference = percent_decoded(reference);
    let mut parts: Vec<&str> = if reference.starts_with('/') {
        Vec::new()
    } else {
        directory
            .split('/')
            .filter(|part| !part.is_empty())
            .collect()
    };
    for part in reference.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    Some(parts.join("/"))
}

/// Passes a chapter's events on, leaving out links into the book's own
/// files (another chapter, a place in this one): their targets are not
/// carried, so the text stays and the link goes.
struct BookLinks<'b, 's> {
    builder: &'b mut DocumentBuilder<'s>,
    /// For each open link, whether it was passed on.
    kept: Vec<bool>,
}

impl<'a> EventSink<'a> for BookLinks<'_, '_> {
    fn event(&mut self, event: Event<'a>) {
        let keep = match &event {
            Event::Start(Tag::Link { destination, .. }) => {
                let keep = is_external(destination);
                self.kept.push(keep);
                keep
            }
            Event::End(TagEnd::Link) => self.kept.pop().unwrap_or(true),
            _ => true,
        };
        if keep {
            self.builder.event(event);
        }
    }

    fn transient(&mut self, event: Event<'_>) {
        self.builder.transient(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn references_resolve_within_the_package() {
        assert_eq!(
            resolve("OEBPS/text", "../images/a%20b.png#x").as_deref(),
            Some("OEBPS/images/a b.png")
        );
        assert_eq!(resolve("", "./cover.jpg").as_deref(), Some("cover.jpg"));
        assert_eq!(resolve("a/b", "/c.png").as_deref(), Some("c.png"));
        assert_eq!(resolve("a", "https://example.com/x.png"), None);
        assert_eq!(resolve("a", "#note-1"), None);
    }
}
