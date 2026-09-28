//! PDF text against pdftotext (poppler): for every fixture in
//! tests/fixtures/pdf-text, each page's lines match the reference's, with
//! runs of whitespace collapsed and blank lines (block breaks, where the
//! two tools may differ) left out. Fixtures and references come from
//! temp/gen_pdf_text_fixtures.py.

use std::fs;
use std::path::PathBuf;

use sublime::io::pdf::text::write_pdf_text;

fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/pdf-text")
}

/// Pages of non-blank lines, whitespace collapsed.
fn pages(text: &str) -> Vec<Vec<String>> {
    let mut pages: Vec<Vec<String>> = text
        .split('\x0c')
        .map(|page| {
            page.lines()
                .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
                .filter(|line| !line.is_empty())
                .collect()
        })
        .collect();
    // Both end with a form feed: the piece after it is empty.
    if pages.last().is_some_and(Vec::is_empty) {
        pages.pop();
    }
    pages
}

/// Fixtures where pdftotext breaks lines differently by design, compared
/// word for word instead, with the reason.
const WORDS_ONLY: &[(&str, &str)] = &[(
    "gs-story.pdf",
    "pdftotext puts each list bullet on a line of its own after Ghostscript \
     re-encodes its font; the bullet and its item share a baseline, so one line",
)];

fn words(pages: &[Vec<String>]) -> Vec<Vec<String>> {
    pages
        .iter()
        .map(|page| {
            page.iter()
                .flat_map(|line| line.split(' ').map(str::to_string))
                .collect()
        })
        .collect()
}

#[test]
fn every_fixture_reads_as_pdftotext_reads_it() {
    let mut names: Vec<String> = fs::read_dir(dir())
        .expect("fixtures")
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.ends_with(".pdf"))
        .collect();
    names.sort();
    assert!(names.len() >= 7, "fixtures missing: {names:?}");
    let mut failures = Vec::new();
    for name in &names {
        let pdf = fs::read(dir().join(name)).expect("pdf");
        let expected =
            fs::read_to_string(dir().join(name.replace(".pdf", ".txt"))).expect("reference");
        let mut out = Vec::new();
        if let Err(error) = write_pdf_text(&pdf, None, &mut out) {
            failures.push(format!("{name}: {error}"));
            continue;
        }
        let got = pages(&String::from_utf8(out).expect("utf-8"));
        let want = pages(&expected);
        let same = if WORDS_ONLY.iter().any(|(fixture, _)| fixture == name) {
            words(&got) == words(&want)
        } else {
            got == want
        };
        if !same {
            failures.push(format!("{name}:\n  got  {got:?}\n  want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn one_page_is_chosen_by_number() {
    let pdf = fs::read(dir().join("multipage.pdf")).expect("pdf");
    let mut out = Vec::new();
    write_pdf_text(&pdf, Some(2), &mut out).expect("page 2");
    assert_eq!(
        pages(&String::from_utf8(out).unwrap()),
        vec![vec!["Page 2 heading", "Body text on page 2."]]
    );
    assert!(write_pdf_text(&pdf, Some(9), &mut Vec::new()).is_err());
}
