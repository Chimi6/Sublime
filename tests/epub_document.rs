//! EPUB into the document model and back. Oracles: pandoc's EPUBs of the
//! Pages fixtures' Word sources read with their notes, pictures, lists,
//! and tables; books built here in Standard Ebooks' and EPUB 2's shapes
//! read in reading order with their endnotes, cover, and only the links
//! that leave the book; and our own EPUB of every Word source reads back
//! to the same Markdown, in a package whose every file is in the manifest
//! and well formed.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::document::{Block, Document, Inline};
use sublime::format::Effort;
use sublime::io::docx::read_docx;
use sublime::io::epub::{read_epub, write_epub};
use sublime::io::markdown::MarkdownWriter;
use sublime::io::xml::{XmlEvent, XmlReader};
use sublime::io::zip::{ZipArchive, ZipWriter};

fn path(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures");
    path.push(relative);
    path
}

fn epub(name: &str) -> Document {
    let bytes = std::fs::read(path(&format!("epub/{name}.epub"))).expect("fixture readable");
    read_epub(&bytes).expect("EPUB reads")
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

#[test]
fn pandocs_notes_are_footnotes() {
    let document = epub("pandoc-notes");
    assert_eq!(document.footnotes.len(), 3);
    let text = markdown(&document);
    assert!(text.contains("[^1]: The first footnote."), "{text}");
    assert!(text.contains("[^3]: The only endnote."), "{text}");
    // The notes' rule and section are markup, not text.
    assert!(!text.contains("---"), "{text}");
}

#[test]
fn pandocs_pictures_lists_and_tables() {
    let document = epub("pandoc-images");
    assert!(!document.images.is_empty());
    for media in &document.media {
        assert!(!media.bytes.is_empty(), "{}", media.name);
    }
    assert!(markdown(&epub("pandoc-lists")).contains("1. "));
    let table = epub("pandoc-table");
    assert!(
        table.sections[0]
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Table(_)))
    );
}

/// A book in Standard Ebooks' shape: a cover named in the manifest, a
/// navigation document and an imprint out of reading order, chapters in
/// a folder, and endnotes in a file of their own with a heading and links
/// back.
fn standard_book() -> Vec<u8> {
    let mut zip = ZipWriter::new(Vec::new());
    zip.add("mimetype", b"application/epub+zip").unwrap();
    zip.add(
        "META-INF/container.xml",
        br#"<?xml version="1.0"?><container version="1.0" xmlns="urn:oasis:names:tc:opendocument:xmlns:container"><rootfiles><rootfile full-path="epub/content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    )
    .unwrap();
    zip.add(
        "epub/content.opf",
        br#"<?xml version="1.0"?><package xmlns="http://www.idpf.org/2007/opf" version="3.0"><metadata/><manifest>
<item href="toc.xhtml" id="toc.xhtml" media-type="application/xhtml+xml" properties="nav"/>
<item href="images/cover.png" id="cover.png" media-type="image/png" properties="cover-image"/>
<item href="text/imprint.xhtml" id="imprint" media-type="application/xhtml+xml"/>
<item href="text/chapter-1.xhtml" id="chapter-1" media-type="application/xhtml+xml"/>
<item href="text/chapter-2.xhtml" id="chapter-2" media-type="application/xhtml+xml"/>
<item href="text/endnotes.xhtml" id="endnotes" media-type="application/xhtml+xml"/>
</manifest><spine><itemref idref="toc.xhtml"/><itemref idref="imprint" linear="no"/><itemref idref="chapter-1"/><itemref idref="chapter-2"/><itemref idref="endnotes"/></spine></package>"#,
    )
    .unwrap();
    zip.add("epub/images/cover.png", b"\x89PNG\r\n\x1a\nnot really")
        .unwrap();
    zip.add(
        "epub/toc.xhtml",
        br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><nav epub:type="toc"><ol><li><a href="text/chapter-1.xhtml">Chapter I</a></li></ol></nav></body></html>"#,
    )
    .unwrap();
    zip.add(
        "epub/text/imprint.xhtml",
        br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><p>Imprint text.</p></body></html>"#,
    )
    .unwrap();
    zip.add(
        "epub/text/chapter-1.xhtml",
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<html xmlns=\"http://www.w3.org/1999/xhtml\" xmlns:epub=\"http://www.idpf.org/2007/ops\"><head><title>Chapter I</title></head><body><section id=\"chapter-1\" epub:type=\"chapter\"><h2>\n\t<span epub:type=\"se:label\">Chapter</span>\n\t<span>I</span>\n</h2><p>A moon rose.<a href=\"endnotes.xhtml#note-1\" id=\"noteref-1\" role=\"doc-noteref\" epub:type=\"noteref\">1</a> See <a href=\"chapter-2.xhtml#chapter-2\">the next chapter</a> and <a href=\"https://example.com/\">the web</a>.</p></section></body></html>"
            .as_bytes(),
    )
    .unwrap();
    zip.add(
        "epub/text/chapter-2.xhtml",
        br#"<html xmlns="http://www.w3.org/1999/xhtml"><body><section id="chapter-2"><h2>Chapter II</h2><p>Night fell.</p></section></body></html>"#,
    )
    .unwrap();
    zip.add(
        "epub/text/endnotes.xhtml",
        br##"<html xmlns="http://www.w3.org/1999/xhtml" xmlns:epub="http://www.idpf.org/2007/ops"><body><section id="endnotes" role="doc-endnotes" epub:type="endnotes"><h2 epub:type="title">Endnotes</h2><ol><li id="note-1" epub:type="endnote"><p>The moon. <a href="chapter-1.xhtml#noteref-1" role="doc-backlink" epub:type="backlink">&#8617;</a></p></li></ol></section></body></html>"##,
    )
    .unwrap();
    zip.finish().unwrap()
}

#[test]
fn a_standard_ebook_reads_in_order_with_its_endnotes() {
    let document = read_epub(&standard_book()).expect("reads");
    let text = markdown(&document);
    // The cover first, then the chapters; not the contents or the imprint.
    assert!(text.starts_with("![](cover.png)"), "{text}");
    assert!(!text.contains("Imprint text"), "{text}");
    assert!(text.contains("## Chapter I\n"), "{text}");
    assert!(text.find("Chapter I\n").unwrap() < text.find("Chapter II").unwrap());
    // The endnote is a footnote, without its way back; its section's
    // heading is gone.
    assert_eq!(document.footnotes.len(), 1);
    assert!(text.contains("A moon rose.[^1]"), "{text}");
    assert!(text.contains("[^1]: The moon."), "{text}");
    assert!(
        !text.contains('\u{21a9}') && !text.contains("Endnotes"),
        "{text}"
    );
    // A link to another chapter keeps its text; a link out stays a link.
    assert!(
        text.contains("See the next chapter and [the web](https://example.com/)"),
        "{text}"
    );
    // Each chapter starts a page.
    let breaks = document.sections[0]
        .blocks
        .iter()
        .filter(|block| match block {
            Block::Paragraph(paragraph) => {
                document.paragraph_properties(paragraph).page_break_before == Some(true)
            }
            Block::Table(_) => false,
        })
        .count();
    assert_eq!(breaks, 2, "chapter 1 after the cover, and chapter 2");
}

/// An EPUB 2 book: no container rootfile prefix, an NCX, and its cover
/// named by a `meta`; the cover page already shows it.
#[test]
fn an_epub2_cover_shown_by_a_page_is_not_doubled() {
    let mut zip = ZipWriter::new(Vec::new());
    zip.add("mimetype", b"application/epub+zip").unwrap();
    zip.add(
        "META-INF/container.xml",
        br#"<container xmlns="urn:oasis:names:tc:opendocument:xmlns:container" version="1.0"><rootfiles><rootfile full-path="content.opf" media-type="application/oebps-package+xml"/></rootfiles></container>"#,
    )
    .unwrap();
    zip.add(
        "content.opf",
        br#"<opf:package xmlns:opf="http://www.idpf.org/2007/opf" version="2.0"><opf:metadata><opf:meta name="cover" content="cov"/></opf:metadata><opf:manifest><opf:item id="cov" href="images/c%20over.jpg" media-type="image/jpeg"/><opf:item id="p1" href="cover.html" media-type="application/xhtml+xml"/><opf:item id="p2" href="body.html" media-type="application/xhtml+xml"/><opf:item id="ncx" href="toc.ncx" media-type="application/x-dtbncx+xml"/></opf:manifest><opf:spine toc="ncx"><opf:itemref idref="p1"/><opf:itemref idref="p2"/></opf:spine></opf:package>"#,
    )
    .unwrap();
    zip.add("images/c over.jpg", b"\xff\xd8\xff\xe0 a picture")
        .unwrap();
    zip.add(
        "cover.html",
        br#"<html><body><div><img src="images/c%20over.jpg" alt="Cover"/></div></body></html>"#,
    )
    .unwrap();
    zip.add("body.html", br#"<html><body><p>Text.</p></body></html>"#)
        .unwrap();
    let document = read_epub(&zip.finish().unwrap()).expect("reads");
    assert_eq!(document.images.len(), 1, "one cover");
    assert_eq!(document.media[0].name, "c over.jpg");
    assert!(markdown(&document).contains("Text."));
}

/// Our EPUB of every Word source reads back to the same Markdown.
#[test]
fn our_epub_of_each_word_source_round_trips() {
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
        let mut source = word(&name);
        let bytes = write_epub(&source, Vec::new(), Effort::Balanced).expect("writes");
        well_formed(&bytes, &name);
        // A book has no place for review comments.
        source.comments.clear();
        for section in &mut source.sections {
            without_comments(&mut section.blocks);
        }
        let back = read_epub(&bytes).expect("reads back");
        assert_eq!(
            visible(&markdown(&back)),
            visible(&markdown(&source)),
            "{name}"
        );
        checked += 1;
    }
    assert!(checked >= 25, "{checked} sources");
}

/// Blocks without the marks of where comments are.
fn without_comments(blocks: &mut [Block]) {
    for block in blocks {
        match block {
            Block::Paragraph(paragraph) => paragraph.runs.retain(|run| {
                !matches!(run.content, Inline::CommentStart(_) | Inline::CommentEnd(_))
            }),
            Block::Table(table) => {
                for row in &mut table.rows {
                    for cell in &mut row.cells {
                        without_comments(&mut cell.blocks);
                    }
                }
            }
        }
    }
}

/// Markdown as a book shows it: without the blank paragraphs a page
/// keeps (an EPUB keeps them as a space), with runs of spaces as one
/// (HTML shows them so), tabs as the em space a book shows for them,
/// and with links to the document's own bookmarks
/// as their text (the model does not carry bookmarks, so a book has no
/// place for them to go).
fn visible(markdown: &str) -> String {
    let mut text = String::with_capacity(markdown.len());
    let mut rest = markdown;
    while let Some(at) = rest.find("](#") {
        let open = rest[..at].rfind('[').unwrap_or(at);
        let close = rest[at..].find(')').map_or(rest.len(), |end| at + end + 1);
        text.push_str(&rest[..open]);
        text.push_str(&rest[open + 1..at]);
        rest = &rest[close..];
    }
    text.push_str(rest);
    let text = text.replace('\t', "\u{2003}");
    text.lines()
        .map(|line| {
            line.split(' ')
                .filter(|word| !word.is_empty())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .filter(|line| !line.is_empty() && line != "\u{a0}")
        .collect::<Vec<_>>()
        .join("\n")
}

/// The package: `mimetype` first and stored, every file named in the
/// manifest, every XHTML and XML part well formed.
fn well_formed(bytes: &[u8], name: &str) {
    assert_eq!(&bytes[30..38], b"mimetype", "{name}: mimetype first");
    assert_eq!(&bytes[38..58], b"application/epub+zip", "{name}: stored");
    let zip = ZipArchive::parse(bytes).expect("a ZIP");
    let read = |part: &str| {
        let mut out = Vec::new();
        zip.read(zip.find(part).expect(part), &mut out)
            .expect("reads");
        String::from_utf8(out).expect("utf-8")
    };
    let opf = read("EPUB/content.opf");
    for entry in zip.entries() {
        let part = &entry.name;
        if part == "mimetype" || part.starts_with("META-INF/") || part == "EPUB/content.opf" {
            continue;
        }
        let href = part.strip_prefix("EPUB/").expect("in EPUB/");
        assert!(
            opf.contains(&format!("href=\"{href}\"")),
            "{name}: {part} not in the manifest"
        );
        if part.ends_with(".xhtml") || part.ends_with(".ncx") || part.ends_with(".opf") {
            let xml = read(part);
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
fn notes_are_pop_up_footnotes_in_their_chapter() {
    let bytes = write_epub(&word("notes"), Vec::new(), Effort::Balanced).expect("writes");
    let zip = ZipArchive::parse(&bytes).expect("a ZIP");
    let mut chapter = Vec::new();
    zip.read(
        zip.find("EPUB/text/ch001.xhtml").expect("a chapter"),
        &mut chapter,
    )
    .unwrap();
    let chapter = String::from_utf8(chapter).unwrap();
    assert!(
        chapter.contains(r##"href="#fn-1" epub:type="noteref""##),
        "{chapter}"
    );
    assert!(chapter.contains(r#"<aside id="fn-1" class="footnote" epub:type="footnote""#));
    let back = read_epub(&bytes).expect("reads");
    assert_eq!(back.footnotes.len(), 3);
}

#[test]
fn every_effort_writes_the_same_book() {
    let source = word("everything");
    let mut texts = Vec::new();
    let mut sizes = Vec::new();
    for effort in [Effort::Fast, Effort::Balanced, Effort::Max] {
        let bytes = write_epub(&source, Vec::new(), effort).expect("writes");
        sizes.push(bytes.len());
        texts.push(markdown(&read_epub(&bytes).expect("reads")));
    }
    assert!(texts.windows(2).all(|pair| pair[0] == pair[1]));
    assert!(sizes[2] <= sizes[1], "{sizes:?}");
}

#[test]
fn pictures_carry_their_bytes() {
    let source = word("images");
    let bytes = write_epub(&source, Vec::new(), Effort::Balanced).expect("writes");
    let back = read_epub(&bytes).expect("reads");
    assert_eq!(back.images.len(), source.images.len());
    for (ours, theirs) in back.media.iter().zip(&source.media) {
        assert_eq!(ours.bytes, theirs.bytes, "{}", theirs.name);
    }
    assert!(
        back.sections[0]
            .blocks
            .iter()
            .any(|block| matches!(block, Block::Paragraph(p) if p.runs.iter().any(|run| matches!(run.content, Inline::Image(_)))))
    );
}
