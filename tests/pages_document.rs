//! The Pages document reader against the fixtures: paragraphs, styles,
//! lists, links, footnotes, and breaks must come out as the fixture README
//! says they went in.

use std::path::PathBuf;

use sublime::document::{
    Alignment, AnchorBase, Baseline, Block, Document, FloatingContent, Inline, ListLabel, Merge,
    NumberKind, Placement, RevisionKind, SectionStart,
};
use sublime::io::pages::{Package, read_document};

fn read(name: &str) -> Document {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/pages");
    path.push(format!("{name}.pages"));
    let bytes = std::fs::read(&path).expect("fixture readable");
    let package = Package::read(&bytes).expect("package reads");
    read_document(&package)
}

fn paragraphs(document: &Document) -> Vec<&sublime::document::Paragraph> {
    document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph),
            Block::Table(_) => None,
        })
        .collect()
}

fn style_name(document: &Document, paragraph: &sublime::document::Paragraph) -> String {
    paragraph
        .style
        .map(|id| document.styles.paragraph[id].name.clone())
        .unwrap_or_default()
}

#[test]
fn text_styles_paragraphs_and_runs() {
    let document = read("text-styles");
    let texts = document.paragraph_texts();
    assert_eq!(texts[0], "Text Styles");
    assert_eq!(texts[2], "Heading One");
    let paragraphs = paragraphs(&document);
    assert_eq!(style_name(&document, paragraphs[0]), "Title");
    assert_eq!(style_name(&document, paragraphs[1]), "Subtitle");
    assert_eq!(style_name(&document, paragraphs[2]), "Heading");
    // The run paragraph: "Plain, bold, italic, ..." checked as effective
    // formatting, since Pages puts some of it in named character styles.
    let runs = &paragraphs[7].runs;
    let find = |needle: &str| {
        let run = runs
            .iter()
            .find(|run| matches!(&run.content, Inline::Text(text) if text.contains(needle)))
            .unwrap_or_else(|| panic!("no run containing {needle:?}"));
        document.effective_run(paragraphs[7], run)
    };
    assert_eq!(find("bold, ").bold, Some(true));
    assert_eq!(find("italic, ").italic, Some(true));
    assert_eq!(find("underlined").underline, Some(true));
    assert_eq!(find("struck").strike, Some(true));
    assert_eq!(
        find("red").color.map(|color| color.hex()),
        Some("FF0000".to_string())
    );
    assert_eq!(find("large").size, Some(18.0));
    assert!(
        find("monospace")
            .font
            .as_deref()
            .is_some_and(|font| font.contains("Courier"))
    );
    assert_eq!(find("script").baseline, Some(Baseline::Superscript));
    assert_ne!(find("Plain, ").bold, Some(true));
    assert!(runs.iter().any(|run| {
        run.style
            .map(|id| document.styles.character[id].name.as_str())
            == Some("Strong")
    }));
    assert!(runs.iter().all(|run| {
        run.style
            .map(|id| document.styles.character[id].name.as_str())
            != Some("None")
    }));
}

#[test]
fn lists_have_levels_and_formats() {
    let document = read("lists");
    let paragraphs = paragraphs(&document);
    let items: Vec<(String, Option<u8>, bool)> = paragraphs
        .iter()
        .map(|paragraph| {
            (
                paragraph.text(),
                paragraph.list.map(|item| item.level),
                paragraph.list.is_some_and(|item| item.starts_list),
            )
        })
        .collect();
    assert_eq!(items[1], ("A bulleted list:".to_string(), None, false));
    assert_eq!(items[2], ("First bullet".to_string(), Some(0), true));
    assert_eq!(
        items[4],
        ("Nested bullet under the second".to_string(), Some(1), false)
    );
    assert_eq!(items[5], ("Third level bullet".to_string(), Some(2), false));
    assert_eq!(items[8], ("First number".to_string(), Some(0), true));
    let numbered = paragraphs[8].list.unwrap();
    let style = &document.styles.list[numbered.style];
    assert!(
        matches!(&style.levels[0].label, ListLabel::Number(format) if format.kind == NumberKind::Decimal)
    );
    assert!(
        matches!(&style.levels[1].label, ListLabel::Number(format) if format.kind == NumberKind::LowerLetter)
    );
    assert!(
        matches!(&style.levels[2].label, ListLabel::Number(format) if format.kind == NumberKind::LowerRoman)
    );
    let bulleted = &document.styles.list[paragraphs[2].list.unwrap().style];
    assert!(matches!(&bulleted.levels[0].label, ListLabel::Text(_)));
}

#[test]
fn paragraph_formatting_and_breaks() {
    let document = read("paragraphs");
    let paragraphs = paragraphs(&document);
    let by_prefix = |prefix: &str| {
        paragraphs
            .iter()
            .find(|paragraph| paragraph.text().starts_with(prefix))
            .unwrap_or_else(|| panic!("no paragraph starting {prefix:?}"))
    };
    assert_eq!(
        by_prefix("Centered").properties.alignment,
        Some(Alignment::Center)
    );
    assert_eq!(
        by_prefix("Right aligned").properties.alignment,
        Some(Alignment::Right)
    );
    assert_eq!(
        by_prefix("Justified").properties.alignment,
        Some(Alignment::Justify)
    );
    assert_eq!(
        by_prefix("Space before").properties.space_before,
        Some(24.0)
    );
    assert_eq!(
        by_prefix("First line indented")
            .properties
            .first_line_indent,
        Some(36.0)
    );
    assert_eq!(
        by_prefix("Keep with next").properties.keep_with_next,
        Some(true)
    );
    let broken = by_prefix("A line break");
    assert!(
        broken
            .runs
            .iter()
            .any(|run| run.content == Inline::LineBreak)
    );
    assert_eq!(broken.text(), "A line break inside\nthe same paragraph.");
    // The page break is a paragraph of its own before the text it leads.
    let after_break = paragraphs
        .iter()
        .position(|paragraph| paragraph.text().starts_with("Text after the page break"))
        .expect("paragraph after the page break");
    let break_paragraph = paragraphs[after_break - 1];
    assert_eq!(break_paragraph.runs.len(), 1);
    assert_eq!(break_paragraph.runs[0].content, Inline::PageBreak);
}

#[test]
fn links_carry_their_targets() {
    let document = read("links");
    let paragraphs = paragraphs(&document);
    let links: Vec<(String, String)> = paragraphs[1]
        .runs
        .iter()
        .filter_map(|run| match (&run.link, &run.content) {
            (Some(url), Inline::Text(text)) => Some((text.clone(), url.clone())),
            _ => None,
        })
        .collect();
    assert_eq!(
        links,
        vec![
            (
                "link to a web page".to_string(),
                "https://example.com/path?query=1%23fragment".to_string()
            ),
            (
                "email link".to_string(),
                "mailto:someone@example.com".to_string()
            ),
        ]
    );
    assert_eq!(
        paragraphs[1].text(),
        "A link to a web page and an email link."
    );
}

#[test]
fn footnotes_are_collected() {
    let document = read("notes");
    let paragraphs = paragraphs(&document);
    let with_notes = paragraphs
        .iter()
        .find(|paragraph| paragraph.text().starts_with("A sentence with a footnote"))
        .expect("footnote paragraph");
    let notes: Vec<usize> = with_notes
        .runs
        .iter()
        .filter_map(|run| match run.content {
            Inline::Footnote(note) => Some(note),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes.len(),
        3,
        "two footnotes and the endnote Pages turned into a footnote"
    );
    let first = &document.footnotes[notes[0]];
    let text = match &first.blocks[0] {
        Block::Paragraph(paragraph) => paragraph.text(),
        _ => String::new(),
    };
    assert_eq!(text, "The first footnote.");
}

fn tables(document: &Document) -> Vec<&sublime::document::Table> {
    document.sections[0]
        .blocks
        .iter()
        .filter_map(|block| match block {
            Block::Table(table) => Some(table),
            Block::Paragraph(_) => None,
        })
        .collect()
}

fn cell_text(cell: &sublime::document::Cell) -> String {
    cell.blocks
        .iter()
        .filter_map(|block| match block {
            Block::Paragraph(paragraph) => Some(paragraph.text()),
            Block::Table(_) => None,
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn tables_come_out_as_grids_with_merges() {
    let document = read("table");
    let tables = tables(&document);
    assert_eq!(tables.len(), 2);
    let simple = tables[0];
    assert_eq!(simple.rows.len(), 3);
    assert_eq!(simple.header_rows, 1);
    assert_eq!(simple.columns, vec![150.0, 150.0, 150.0]);
    let texts: Vec<String> = simple.rows[0].cells.iter().map(cell_text).collect();
    assert_eq!(texts, ["Name", "Kind", "Amount"]);
    assert_eq!(cell_text(&simple.rows[1].cells[2]), "42.50");
    assert!(simple.rows[0].cells[0].background.is_some());
    let merged = tables[1];
    assert_eq!(merged.rows.len(), 4);
    let wide = &merged.rows[0].cells[0];
    assert_eq!(cell_text(wide), "Spans two columns");
    assert_eq!(wide.column_span, 2);
    assert_eq!(merged.rows[0].cells[1].merge, Merge::Left);
    let tall = &merged.rows[1].cells[0];
    assert_eq!(cell_text(tall), "Spans two rows");
    assert_eq!(tall.row_span, 2);
    assert_eq!(merged.rows[2].cells[0].merge, Merge::Above);
    assert_eq!(
        cell_text(&merged.rows[3].cells[0]),
        "Multi-line cell\nsecond paragraph"
    );
    // The shaded cell has its own fill, unlike its neighbours.
    assert_ne!(wide.background, merged.rows[0].cells[2].background);
    // The table sits before the paragraph that held it.
    let texts = document.paragraph_texts();
    assert_eq!(
        texts[1],
        "A simple table with a header row and a numeric column:"
    );
    assert_eq!(texts[2], "Name");
}

#[test]
fn images_carry_media_size_and_placement() {
    let document = read("images");
    assert_eq!(document.media.len(), 2);
    assert!(document.media[0].bytes.starts_with(b"\x89PNG"));
    let images: Vec<&sublime::document::InlineImage> = paragraphs(&document)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| match &run.content {
            Inline::Image(image) => Some(image),
            _ => None,
        })
        .collect();
    assert_eq!(images.len(), 3);
    assert_eq!((images[0].width, images[0].height), (72.0, 48.0));
    assert_eq!(images[0].description.as_deref(), Some("inline.png"));
    assert_eq!(images[0].placement, Placement::Inline);
    match images[1].placement {
        Placement::Floating { horizontal, .. } => {
            assert_eq!(horizontal.from, AnchorBase::Page);
            assert_eq!(horizontal.offset, 492.5);
        }
        Placement::Inline => panic!("second image floats"),
    }
    // The scaled copy reuses the first image's media.
    assert_eq!(images[2].media, images[0].media);
    assert_eq!((images[2].width, images[2].height), (144.0, 96.0));
}

fn area_text(blocks: &Option<Vec<Block>>) -> String {
    blocks
        .as_ref()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| match block {
                    Block::Paragraph(paragraph) => Some(paragraph.text()),
                    Block::Table(_) => None,
                })
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

#[test]
fn headers_footers_and_page_setup() {
    let document = read("headers");
    let section = &document.sections[0];
    assert_eq!((section.page.width, section.page.height), (612.0, 792.0));
    assert_eq!(section.page.margin_left, 72.0);
    assert_eq!(section.page.header_distance, 35.4);
    assert_eq!(
        area_text(&section.headers.default),
        "Odd-page header, shown on pages one and three"
    );
    assert_eq!(
        area_text(&section.headers.first),
        "First-page header, shown only on page one"
    );
    assert_eq!(
        area_text(&section.headers.even),
        "Even-page header, shown on page two"
    );
    let footer = section.footers.default.as_ref().expect("footer");
    let Block::Paragraph(paragraph) = &footer[0] else {
        panic!("footer paragraph");
    };
    assert_eq!(paragraph.text(), "Page ");
    assert!(
        paragraph
            .runs
            .iter()
            .any(|run| run.content == Inline::PageNumber)
    );
}

#[test]
fn sections_split_at_section_and_layout_breaks() {
    let document = read("layout");
    assert_eq!(document.sections.len(), 2);
    assert_eq!(document.sections[0].columns, 1);
    assert_eq!(document.sections[1].columns, 2);
    assert_eq!(document.sections[1].start, SectionStart::NewPage);
    assert_eq!(
        area_text(&document.sections[1].headers.default),
        "Section two header, two columns"
    );
    let document = read("page-layout");
    assert_eq!(document.sections.len(), 2);
    assert_eq!(document.sections[0].columns, 2);
}

#[test]
fn floating_text_boxes_and_images_are_collected() {
    let document = read("native-objects");
    assert_eq!(document.floating.len(), 3);
    let lone = &document.floating[2];
    assert_eq!(
        (lone.x, lone.y, lone.width, lone.height),
        (72.0, 260.0, 220.0, 90.0)
    );
    match &lone.content {
        FloatingContent::TextBox { blocks, .. } => {
            assert_eq!(area_text(&Some(blocks.clone())), "A lone shape with text.");
        }
        FloatingContent::Image(_) => panic!("a text box"),
    }
    // Grouped shapes are placed relative to their group.
    assert_eq!(
        (document.floating[1].x, document.floating[1].y),
        (220.0, 130.0)
    );
    let document = read("native-scripted");
    assert_eq!(document.floating.len(), 2);
    assert!(matches!(
        document.floating[0].content,
        FloatingContent::Image(_)
    ));
}

#[test]
fn table_of_contents_entries_follow_their_paragraph() {
    let document = read("toc");
    let paragraphs = paragraphs(&document);
    assert_eq!(style_name(&document, paragraphs[3]), "TOC 1");
    assert_eq!(paragraphs[3].text(), "Contents\t1");
    assert_eq!(style_name(&document, paragraphs[6]), "TOC 2");
    assert_eq!(paragraphs[6].text(), "A subsection\t3");
    assert_eq!(style_name(&document, paragraphs[9]), "Heading");
}

#[test]
fn tracked_changes_are_revisions() {
    let document = read("notes");
    let changed = paragraphs(&document)
        .into_iter()
        .find(|paragraph| paragraph.text().starts_with("Tracked changes"))
        .expect("tracked paragraph");
    assert_eq!(
        changed.text(),
        "Tracked changes: inserted words and unchanged words."
    );
    let revisions: Vec<(RevisionKind, &str)> = changed
        .runs
        .iter()
        .filter_map(|run| {
            let revision = run.revision.as_ref()?;
            Some((revision.kind, revision.author.as_deref().unwrap_or("")))
        })
        .collect();
    assert_eq!(
        revisions,
        [
            (RevisionKind::Insertion, "Editor"),
            (RevisionKind::Deletion, "Editor")
        ]
    );
}

#[test]
fn equations_keep_their_mathml() {
    let document = read("equations");
    let math: Vec<&String> = paragraphs(&document)
        .iter()
        .flat_map(|paragraph| paragraph.runs.iter())
        .filter_map(|run| match &run.content {
            Inline::Math(mathml) => Some(mathml),
            _ => None,
        })
        .collect();
    assert_eq!(math.len(), 4);
    assert!(math[0].starts_with("<math"));
    assert_eq!(sublime::document::mathml_text(math[0]), "E=mc2");
}
