//! OpenDocument Text into the document model and back. Oracles: ODT
//! written by LibreOffice (the reference OpenDocument implementation) from
//! the Pages fixtures' Word sources reads to the same Markdown as those
//! Word files, with their notes, comments, changes, pictures, and pages;
//! and our own ODT of every Word source reads back to the same Markdown,
//! in a package every part of which is well formed.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::document::{Document, Inline, RevisionKind};
use sublime::format::Effort;
use sublime::io::docx::read_docx;
use sublime::io::markdown::MarkdownWriter;
use sublime::io::odt::{read_odt, write_odt};
use sublime::io::xml::{XmlEvent, XmlReader};
use sublime::io::zip::ZipArchive;

fn path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(relative);
    path
}

fn odt(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("odt/{name}.odt"))).expect("fixture readable");
    read_odt(&bytes).expect("ODT reads")
}

fn word(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("pages/sources/{name}.docx"))).expect("source");
    read_docx(&bytes).expect("Word reads")
}

fn markdown(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// LibreOffice's ODT of each source reads as the source does.
#[test]
fn libreoffice_odt_reads_as_its_word_source() {
    for name in [
        "lists",
        "headers",
        "links",
        "page-layout",
        "paragraphs",
        "tabs",
        "custom-styles",
        "notes",
    ] {
        assert_eq!(
            markdown(&odt(&format!("lo-{name}"))),
            markdown(&word(name)),
            "{name}"
        );
    }
}

#[test]
fn notes_and_endnotes_read_as_footnotes() {
    let document = odt("lo-notes");
    assert_eq!(document.footnotes.len(), 3);
    let text = markdown(&document);
    assert!(text.contains("[^1]: The first footnote."), "{text}");
    assert!(text.contains("[^3]: The only endnote."), "{text}");
}

#[test]
fn a_comment_thread_and_tracked_changes() {
    let document = odt("lo-revisions");
    let replies: Vec<_> = document
        .comments
        .iter()
        .filter(|comment| comment.reply_to.is_some())
        .collect();
    assert_eq!(replies.len(), 1, "one reply");
    let parent = replies[0].reply_to.expect("a parent") as usize;
    assert_eq!(document.comments[parent].author, "Reviewer");
    let kinds: Vec<RevisionKind> = document
        .revisions
        .iter()
        .map(|revision| revision.kind)
        .collect();
    assert!(kinds.contains(&RevisionKind::Insertion), "{kinds:?}");
    // A deletion's text is kept, marked deleted, where it was.
    let document = odt("lo-notes");
    assert!(
        document
            .revisions
            .iter()
            .any(|revision| revision.kind == RevisionKind::Deletion)
    );
    let deleted: String = document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            sublime::document::Block::Paragraph(paragraph) => Some(paragraph),
            _ => None,
        })
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| document.is_deleted(run))
        .filter_map(|run| match run.content {
            Inline::Text(span) => Some(document.text(span).to_string()),
            _ => None,
        })
        .collect();
    assert!(!deleted.trim().is_empty());
}

#[test]
fn pictures_are_read_whole() {
    let document = odt("lo-images");
    let source = word("images");
    assert_eq!(document.images.len(), source.images.len());
    for media in &document.media {
        assert!(
            media.bytes.starts_with(&[0x89, b'P', b'N', b'G']),
            "{}",
            media.name
        );
    }
    // Sized as Word sized them, within LibreOffice's rounding.
    for (ours, theirs) in document.images.iter().zip(&source.images) {
        assert!(
            (ours.width - theirs.width).abs() < 1.0,
            "{} {}",
            ours.width,
            theirs.width
        );
        assert!((ours.height - theirs.height).abs() < 1.0);
    }
}

/// A table's merged cells, as LibreOffice wrote Word's (it states the
/// cells' alignment, which the Word file left to its default).
#[test]
fn merged_cells_are_covered() {
    use sublime::document::{Block, Merge};
    let document = odt("lo-table");
    let tables: Vec<_> = document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            _ => None,
        })
        .collect();
    assert_eq!(tables.len(), 2);
    let merged = tables[1];
    assert_eq!(merged.rows[0].cells[0].column_span, 2);
    assert_eq!(merged.rows[0].cells[1].merge, Merge::Left);
    assert_eq!(merged.rows[1].cells[0].row_span, 2);
    assert_eq!(merged.rows[2].cells[0].merge, Merge::Above);
    assert!(
        merged
            .rows
            .iter()
            .flat_map(|row| &row.cells)
            .any(|cell| cell.background.is_some())
    );
}

#[test]
fn pages_and_headers() {
    let document = odt("lo-page-layout");
    let source = word("page-layout");
    let (ours, theirs) = (&document.sections[0].page, &source.sections[0].page);
    assert!(
        (ours.width - theirs.width).abs() < 1.0,
        "{} {}",
        ours.width,
        theirs.width
    );
    assert!((ours.height - theirs.height).abs() < 1.0);
    assert!((ours.margin_left - theirs.margin_left).abs() < 1.0);
    let headers = odt("lo-headers");
    assert!(
        headers
            .sections
            .iter()
            .any(|section| !section.headers.is_empty())
    );
    assert!(
        headers
            .sections
            .iter()
            .any(|section| !section.footers.is_empty())
    );
}

/// Our ODT of every Word source reads back to the same Markdown.
#[test]
fn our_odt_of_each_word_source_round_trips() {
    let directory = path("pages/sources");
    let mut names: Vec<String> = std::fs::read_dir(&directory)
        .expect("sources")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().into_string().ok()?;
            name.strip_suffix(".docx").map(str::to_string)
        })
        .collect();
    names.sort();
    let mut checked = 0;
    for name in names {
        let source = word(&name);
        let bytes = write_odt(&source, Vec::new(), Effort::Balanced).expect("writes");
        let back = read_odt(&bytes).expect("reads back");
        assert_eq!(markdown(&back), markdown(&source), "{name}");
        assert_eq!(
            page_parts(&back),
            page_parts(&source),
            "{name}: headers and footers"
        );
        well_formed(&bytes, &name);
        checked += 1;
    }
    assert!(checked >= 25, "{checked} sources");
}

/// Which headers and footers each section has.
fn page_parts(document: &Document) -> Vec<[bool; 6]> {
    document
        .sections
        .iter()
        .map(|section| {
            let (headers, footers) = (&section.headers, &section.footers);
            [
                headers.default.is_some(),
                headers.first.is_some(),
                headers.even.is_some(),
                footers.default.is_some(),
                footers.first.is_some(),
                footers.even.is_some(),
            ]
        })
        .collect()
}

/// The package: `mimetype` first and stored, every part named in the
/// manifest, and every XML part's tags balanced.
fn well_formed(bytes: &[u8], name: &str) {
    assert_eq!(&bytes[30..38], b"mimetype", "{name}: mimetype first");
    assert_eq!(
        &bytes[38..77],
        b"application/vnd.oasis.opendocument.text",
        "{name}: stored"
    );
    let zip = ZipArchive::parse(bytes).expect("a ZIP");
    let mut manifest = Vec::new();
    zip.read(
        zip.find("META-INF/manifest.xml").expect("a manifest"),
        &mut manifest,
    )
    .expect("reads");
    let manifest = String::from_utf8(manifest).expect("utf-8");
    for entry in zip.entries() {
        let part = &entry.name;
        if part == "mimetype" || part == "META-INF/manifest.xml" {
            continue;
        }
        assert!(
            manifest.contains(&format!("manifest:full-path=\"{part}\"")),
            "{name}: {part} not in the manifest"
        );
        if part.ends_with(".xml") {
            let mut xml = Vec::new();
            zip.read(entry, &mut xml).expect("reads");
            let xml = String::from_utf8(xml).expect("utf-8");
            let mut open: Vec<&str> = Vec::new();
            for event in XmlReader::new(&xml) {
                match event {
                    XmlEvent::Start {
                        name: tag,
                        self_closing: false,
                        ..
                    } => open.push(tag),
                    XmlEvent::End { name: tag } => {
                        assert_eq!(open.pop(), Some(tag), "{name}: {part} unbalanced");
                    }
                    _ => {}
                }
            }
            assert!(open.is_empty(), "{name}: {part} left {open:?} open");
        }
    }
}

#[test]
fn every_effort_writes_the_same_document() {
    let source = word("everything");
    let mut texts = Vec::new();
    let mut sizes = Vec::new();
    for effort in [Effort::Fast, Effort::Balanced, Effort::Max] {
        let bytes = write_odt(&source, Vec::new(), effort).expect("writes");
        sizes.push(bytes.len());
        texts.push(markdown(&read_odt(&bytes).expect("reads")));
    }
    assert!(texts.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(sizes[2] <= sizes[1], "{sizes:?}");
}

#[test]
fn the_page_colour_carries() {
    let mut source = word("paragraphs");
    let color = sublime::document::Color {
        red: 0xfa,
        green: 0xf0,
        blue: 0xe6,
    };
    source.page_color = Some(color);
    let bytes = write_odt(&source, Vec::new(), Effort::Balanced).expect("writes");
    assert_eq!(read_odt(&bytes).expect("reads").page_color, Some(color));
}
