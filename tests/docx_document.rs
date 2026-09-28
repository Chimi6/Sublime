//! Word input against two independent sources: Apple's own Word exports
//! of the Pages fixtures must project to the same text as the Pages
//! documents themselves, and our Word output must read back to the same
//! Markdown the Pages document gives directly.

use std::path::PathBuf;

use sublime::document::markdown::emit_events;
use sublime::document::{Block, Document, Inline, Merge, RevisionKind};
use sublime::io::docx::{read_docx, write_docx};
use sublime::io::markdown::MarkdownWriter;
use sublime::io::pages::{Package, Scope, read_document};
use sublime::io::text::TextWriter;

fn fixture(relative: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(relative);
    path
}

fn pages_document(name: &str) -> Document {
    let bytes = std::fs::read(fixture(&format!("{name}.pages"))).expect("fixture readable");
    let package = Package::read_scope(&bytes, Scope::Document).expect("package reads");
    read_document(&package)
}

fn apple_document(name: &str) -> Document {
    let bytes =
        std::fs::read(fixture(&format!("reference/{name}.docx"))).expect("reference readable");
    read_docx(&bytes).expect("Word package reads")
}

fn markdown(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = MarkdownWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

fn text(document: &Document) -> String {
    let mut output = Vec::new();
    let mut writer = TextWriter::streaming(&mut output);
    emit_events(document, &mut writer);
    writer.finish().expect("writes");
    String::from_utf8(output).expect("utf-8")
}

/// Words of a text projection. Internal link targets are left out: Pages
/// names its bookmarks by UUID and Apple's export renames them.
fn words(text: &str) -> Vec<String> {
    text.split_whitespace()
        .filter(|word| !word.starts_with("(#"))
        .map(str::to_string)
        .collect()
}

fn first_difference(name: &str, what: &str, ours: &[String], reference: &[String]) {
    for (index, (left, right)) in ours.iter().zip(reference).enumerate() {
        assert_eq!(
            left, right,
            "{name}: {what} differ at word {index} (Word input vs Pages input)"
        );
    }
    assert_eq!(ours.len(), reference.len(), "{name}: {what} word count");
}

/// Fixtures whose Apple export carries the same text as the Pages
/// document, word for word. Left out: `dropcap` (the dropped letter is a
/// paragraph of its own in the export), `ruby` (the base text is
/// dropped), `equations` (Office Math loses operator glyphs), `toc`
/// (an extra empty paragraph), and the `native-*` fixtures (text boxes
/// anchored to different paragraphs).
const TEXT_FIXTURES: &[&str] = &[
    "alt-text",
    "custom-styles",
    "everything",
    "fields",
    "fonts",
    "headers",
    "images",
    "layout",
    "links",
    "lists",
    "metadata",
    "notes",
    "outline-numbering",
    "page-layout",
    "paragraphs",
    "revisions",
    "rtl",
    "table",
    "table-layout",
    "tabs",
    "text-effects",
    "text-styles",
];

/// Every fixture but `equations`: the Word writer puts equations in as
/// text until Office Math is written, so the read-back is text, not math.
const ROUND_TRIP_FIXTURES: &[&str] = &[
    "alt-text",
    "custom-styles",
    "dropcap",
    "everything",
    "fields",
    "fonts",
    "headers",
    "images",
    "layout",
    "links",
    "lists",
    "metadata",
    "native-objects",
    "native-scripted",
    "notes",
    "outline-numbering",
    "page-layout",
    "paragraphs",
    "revisions",
    "rtl",
    "ruby",
    "table",
    "table-layout",
    "tabs",
    "text-effects",
    "text-styles",
    "toc",
];

#[test]
fn apples_export_reads_to_the_pages_text() {
    for name in TEXT_FIXTURES {
        let from_word = words(&text(&apple_document(name)));
        let from_pages = words(&text(&pages_document(name)));
        first_difference(name, "texts", &from_word, &from_pages);
    }
}

#[test]
fn our_word_output_reads_back_to_the_same_markdown() {
    for name in ROUND_TRIP_FIXTURES {
        let mut document = pages_document(name);
        // The Word package names media by index; the reader can only see
        // those names.
        for (index, media) in document.media.iter_mut().enumerate() {
            let extension = media.name.rsplit('.').next().unwrap_or("").to_string();
            media.name = format!("image{}.{extension}", index + 1);
        }
        let direct = markdown(&document);
        let docx = write_docx(&document, Vec::new()).expect("docx writes");
        let read_back = read_docx(&docx).expect("our docx reads");
        let through_word = markdown(&read_back);
        assert_eq!(
            through_word, direct,
            "{name}: pages -> docx -> markdown differs from pages -> markdown"
        );
    }
}

fn body_paragraphs(document: &Document) -> Vec<&sublime::document::Paragraph> {
    let mut paragraphs = Vec::new();
    for section in &document.sections {
        for block in &section.blocks {
            if let Block::Paragraph(paragraph) = block {
                paragraphs.push(paragraph);
            }
        }
    }
    paragraphs
}

#[test]
fn lists_carry_levels_and_kinds() {
    let document = apple_document("lists");
    let items: Vec<(u8, bool)> = body_paragraphs(&document)
        .iter()
        .filter_map(|paragraph| paragraph.list)
        .map(|item| {
            let ordered = document.styles.list[item.style]
                .levels
                .get(usize::from(item.level))
                .is_some_and(|level| {
                    matches!(level.label, sublime::document::ListLabel::Number(_))
                });
            (item.level, ordered)
        })
        .collect();
    assert!(items.len() >= 10, "list paragraphs: {}", items.len());
    assert!(items.contains(&(0, false)), "a top-level bullet");
    assert!(items.contains(&(1, false)), "a nested bullet");
    assert!(items.contains(&(2, false)), "a third-level bullet");
    assert!(items.contains(&(0, true)), "a numbered item");
    let markdown = markdown(&document);
    assert!(markdown.contains("- First bullet\n"), "{markdown}");
    assert!(
        markdown.contains("  - Nested bullet under the second\n"),
        "{markdown}"
    );
    assert!(markdown.contains("1. "), "{markdown}");
}

#[test]
fn tables_keep_their_grid_and_merges() {
    let document = apple_document("table");
    let tables: Vec<&sublime::document::Table> = document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            Block::Paragraph(_) => None,
        })
        .collect();
    assert!(!tables.is_empty());
    let mut saw_left = false;
    let mut saw_above = false;
    for table in &tables {
        let width = table.rows[0].cells.len();
        for row in &table.rows {
            assert_eq!(row.cells.len(), width, "every row spans the grid");
            for cell in &row.cells {
                saw_left |= cell.merge == Merge::Left;
                saw_above |= cell.merge == Merge::Above;
            }
        }
    }
    assert!(saw_left, "a horizontally merged cell");
    assert!(saw_above, "a vertically merged cell");
    assert!(
        tables.iter().any(|table| table.header_rows == 1),
        "a header row"
    );
}

#[test]
fn notes_links_images_and_revisions_are_read() {
    let notes = apple_document("notes");
    assert_eq!(notes.footnotes.len(), 3, "three footnotes");
    let references = body_paragraphs(&notes)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| matches!(run.content, Inline::Footnote(_)))
        .count();
    assert_eq!(references, 3);

    let links = apple_document("links");
    assert!(
        links
            .links
            .iter()
            .any(|target| target.starts_with("https://example.com")),
        "{:?}",
        links.links
    );
    assert!(
        links
            .links
            .iter()
            .any(|target| target.starts_with("mailto:"))
    );
    let markdown = markdown(&links);
    assert!(
        markdown.contains("[link to a web page](https://example.com/"),
        "{markdown}"
    );

    let images = apple_document("images");
    assert!(!images.media.is_empty(), "media parts are loaded");
    assert!(!images.images.is_empty(), "inline images are read");

    // Apple's export keeps insertions and drops deletions.
    let revisions = apple_document("revisions");
    assert!(
        revisions
            .revisions
            .iter()
            .any(|revision| revision.kind == RevisionKind::Insertion)
    );
    let inserted = body_paragraphs(&revisions)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter(|run| run.revision.is_some())
        .count();
    assert!(inserted > 0, "runs carry their revision");
}

#[test]
fn headers_and_page_setup_are_read() {
    let document = apple_document("headers");
    let section = &document.sections[0];
    assert!(section.headers.default.is_some(), "a default header");
    assert!(section.footers.default.is_some(), "a default footer");
    assert!(section.headers.first.is_some(), "a first-page header");
    assert!(
        (section.page.width - 612.0).abs() < 0.5,
        "{}",
        section.page.width
    );
    assert!((section.page.margin_left - 72.0).abs() < 0.5);
}

#[test]
fn headings_come_from_outline_levels() {
    let document = apple_document("paragraphs");
    let markdown = markdown(&document);
    assert!(markdown.starts_with("# "), "{markdown}");
}

/// The comment range markers in a document's body, in order.
fn comment_markers(document: &Document) -> Vec<(bool, u32)> {
    document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| match run.content {
            Inline::CommentStart(id) => Some((true, id)),
            Inline::CommentEnd(id) => Some((false, id)),
            _ => None,
        })
        .collect()
}

/// Word comments keep their author, date, text, and range through our Word
/// output.
#[test]
fn comments_round_trip_through_word() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let document = read_docx(&bytes).expect("Word package reads");
    assert_eq!(document.comments.len(), 2);
    assert_eq!(document.comments[0].author, "Reviewer");
    assert_eq!(
        document.comments[0].text,
        "This is a comment on the word 'commented'."
    );
    assert_eq!(
        document.comments[0].date.as_deref(),
        Some("2026-09-23T12:00:00Z")
    );
    let markers = comment_markers(&document);
    assert_eq!(markers, [(true, 0), (false, 0), (true, 1), (false, 1)]);

    let written = write_docx(&document, Vec::new()).expect("writes");
    let round = read_docx(&written).expect("our Word output reads");
    assert_eq!(round.comments, document.comments);
    assert_eq!(comment_markers(&round), markers);
}

/// The text each comment covers, in the order the comments start.
fn commented_text(document: &Document) -> Vec<(u32, String)> {
    let mut open: Vec<(u32, String)> = Vec::new();
    let mut done = Vec::new();
    for paragraph in document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
    {
        for run in &paragraph.runs {
            match run.content {
                Inline::CommentStart(id) => open.push((id, String::new())),
                Inline::CommentEnd(id) => {
                    if let Some(at) = open.iter().position(|(open, _)| *open == id) {
                        done.push(open.remove(at));
                    }
                }
                Inline::Text(span) => {
                    for (_, text) in &mut open {
                        text.push_str(document.text(span));
                    }
                }
                _ => {}
            }
        }
    }
    done
}

/// Apple's Pages comments read back with their author, text, and range, and
/// reach Word.
#[test]
fn pages_comments_read_with_their_ranges() {
    let document = pages_document("notes");
    assert_eq!(document.comments.len(), 2);
    assert_eq!(document.comments[0].author, "Reviewer");
    assert_eq!(
        document.comments[1].text,
        "A second comment on a whole sentence."
    );
    assert!(document.comments[0].date.is_some());
    let covered = commented_text(&document);
    assert_eq!(covered[0].1, "commented");
    assert_eq!(covered[1].1, "This whole sentence carries a comment.");

    let written = write_docx(&document, Vec::new()).expect("writes");
    let round = read_docx(&written).expect("our Word output reads");
    assert_eq!(round.comments.len(), 2);
    assert_eq!(commented_text(&round), covered);
}

/// The text of each note in the order the body refers to them.
fn referenced_notes(document: &Document) -> Vec<String> {
    document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| match run.content {
            Inline::Footnote(note) => document.footnotes.get(note),
            _ => None,
        })
        .map(|note| {
            note.blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(document.paragraph_text(paragraph)),
                    Block::Table(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim()
                .to_string()
        })
        .collect()
}

/// Footnotes and endnotes number apart in Word, so an endnote's reference
/// finds the endnote, and all of them reach Pages as footnotes.
#[test]
fn notes_resolve_and_reach_pages() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let document = read_docx(&bytes).expect("Word package reads");
    let notes = referenced_notes(&document);
    assert_eq!(
        notes,
        [
            "The first footnote.",
            "The second footnote, with a link-free sentence.",
            "The only endnote."
        ]
    );
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    let round = read_document(&package);
    assert_eq!(referenced_notes(&round), notes);
}

/// Word comments reach Pages as highlights with their comment storage, and
/// read back with the same text, author, and range.
#[test]
fn comments_reach_pages() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let document = read_docx(&bytes).expect("Word package reads");
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    let round = read_document(&package);
    assert_eq!(round.comments.len(), 2);
    for (ours, source) in round.comments.iter().zip(&document.comments) {
        assert_eq!(ours.author, source.author);
        assert_eq!(ours.text, source.text);
        assert_eq!(ours.date, source.date);
    }
    assert_eq!(commented_text(&round), commented_text(&document));
}

/// Tracked insertions and deletions: (kind, author, text), in order.
fn tracked(document: &Document) -> Vec<(RevisionKind, Option<String>, String)> {
    let mut out: Vec<(RevisionKind, Option<String>, String)> = Vec::new();
    for paragraph in document
        .sections
        .iter()
        .flat_map(|section| section.blocks.iter())
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
    {
        for run in &paragraph.runs {
            let (Some(revision), Inline::Text(span)) = (document.revision(run), run.content) else {
                continue;
            };
            let text = document.text(span);
            match out.last_mut() {
                Some((kind, author, words))
                    if *kind == revision.kind && *author == revision.author =>
                {
                    words.push_str(text)
                }
                _ => out.push((revision.kind, revision.author.clone(), text.to_string())),
            }
        }
    }
    out
}

/// Word's tracked changes reach Pages as tracked changes, deleted text kept.
#[test]
fn tracked_changes_reach_pages() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let document = read_docx(&bytes).expect("Word package reads");
    let changes = tracked(&document);
    assert_eq!(changes.len(), 2, "{changes:?}");
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    let round = read_document(&package);
    assert_eq!(tracked(&round), changes);
}

/// A page colour reaches Pages as the sections' background and Word as the
/// document background it is told to show.
#[test]
fn page_color_reaches_both_formats() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let mut document = read_docx(&bytes).expect("Word package reads");
    let yellow = sublime::document::Color {
        red: 255,
        green: 255,
        blue: 204,
    };
    document.page_color = Some(yellow);
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    assert_eq!(read_document(&package).page_color, Some(yellow));
    let word = write_docx(&document, Vec::new()).expect("writes Word");
    assert_eq!(read_docx(&word).expect("reads").page_color, Some(yellow));
}

/// A drawing a header repeats on its pages comes back from our Word output
/// in that header, not on one page of the body.
#[test]
fn header_drawings_stay_in_their_header() {
    use sublime::document::{
        FloatingContent, FloatingObject, PageKind, PagePart, Paragraph, TextWrap,
    };
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let mut document = read_docx(&bytes).expect("Word package reads");
    let part = PagePart {
        footer: false,
        pages: PageKind::Default,
    };
    document.sections[0].headers.default = Some(vec![Block::Paragraph(Paragraph::default())]);
    document.floating.push(FloatingObject {
        page: 0,
        x: 36.0,
        y: 20.0,
        width: 100.0,
        height: 40.0,
        content: FloatingContent::TextBox {
            blocks: Vec::new(),
            fill: Some(sublime::document::Color {
                red: 200,
                green: 0,
                blue: 0,
            }),
            line: None,
            geometry: Default::default(),
            flip: (false, false),
            ends: (None, None),
        },
        follows_text: false,
        wrap: TextWrap::None,
        repeats: Some(part),
        behind: false,
    });
    let written = write_docx(&document, Vec::new()).expect("writes");
    let round = read_docx(&written).expect("reads");
    let repeated: Vec<_> = round
        .floating
        .iter()
        .filter(|object| object.repeats == Some(part))
        .collect();
    assert_eq!(repeated.len(), 1);
    assert!((repeated[0].x - 36.0).abs() < 0.5 && (repeated[0].y - 20.0).abs() < 0.5);
}

/// A comment on text Pages keeps no comments in (a header here) is kept,
/// on the body's first character, rather than lost.
#[test]
fn comments_outside_the_body_are_kept() {
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let mut document = read_docx(&bytes).expect("Word package reads");
    // Move the paragraph holding the first comment into a header.
    let blocks = &mut document.sections[0].blocks;
    let at = blocks
        .iter()
        .position(|block| match block {
            Block::Paragraph(paragraph) => paragraph
                .runs
                .iter()
                .any(|run| run.content == Inline::CommentStart(0)),
            Block::Table(_) => false,
        })
        .expect("a commented paragraph");
    let moved = blocks.remove(at);
    document.sections[0].headers.default = Some(vec![moved]);
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    let round = read_document(&package);
    assert_eq!(round.comments.len(), 2);
    assert!(
        round
            .comments
            .iter()
            .any(|comment| comment.text == document.comments[0].text)
    );
}

/// A drawing behind the text stays behind it through Pages and Word.
#[test]
fn behind_text_drawings_stay_behind() {
    use sublime::document::{FloatingContent, FloatingObject, TextWrap};
    let bytes = std::fs::read(fixture("sources/notes.docx")).expect("source readable");
    let mut document = read_docx(&bytes).expect("Word package reads");
    for behind in [true, false] {
        document.floating.push(FloatingObject {
            page: 0,
            x: 72.0 + if behind { 0.0 } else { 200.0 },
            y: 72.0,
            width: 150.0,
            height: 60.0,
            content: FloatingContent::TextBox {
                blocks: Vec::new(),
                fill: Some(sublime::document::Color {
                    red: 230,
                    green: 230,
                    blue: 250,
                }),
                line: None,
                geometry: Default::default(),
                flip: (false, false),
                ends: (None, None),
            },
            follows_text: false,
            wrap: TextWrap::None,
            repeats: None,
            behind,
        });
    }
    let flags = |document: &Document| {
        let mut flags: Vec<bool> = document
            .floating
            .iter()
            .map(|object| object.behind)
            .collect();
        flags.sort();
        flags
    };
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    assert_eq!(flags(&read_document(&package)), [false, true]);
    let word = write_docx(&document, Vec::new()).expect("writes Word");
    assert_eq!(flags(&read_docx(&word).expect("reads")), [false, true]);
}

/// A Word comment thread (a reply to a comment) reaches Pages as the
/// comment's replies, and comes back to Word as the same thread.
#[test]
fn comment_threads_survive_both_ways() {
    let bytes = std::fs::read(fixture("sources/revisions.docx")).expect("source readable");
    let document = read_docx(&bytes).expect("Word package reads");
    let thread = |document: &Document| {
        document
            .comments
            .iter()
            .map(|comment| {
                (
                    comment.author.clone(),
                    comment.text.clone(),
                    comment.reply_to,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(
        thread(&document)
            .iter()
            .map(|(_, _, parent)| *parent)
            .collect::<Vec<_>>(),
        [None, Some(0)]
    );
    let pages = sublime::io::pages::write_package(&document).expect("writes Pages");
    let package = Package::read_scope(&pages, Scope::Document).expect("our package reads");
    let round = read_document(&package);
    assert_eq!(thread(&round), thread(&document));
    let word = write_docx(&round, Vec::new()).expect("writes Word");
    assert_eq!(thread(&read_docx(&word).expect("reads")), thread(&document));
}
