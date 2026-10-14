//! EPUB, the e-book format: a ZIP of XHTML chapters, their pictures,
//! styles, and fonts, with a package document that lists them and orders
//! the chapters.

pub mod reader;
pub mod writer;

pub use reader::{EpubError, read_epub};
pub use writer::write_epub;

/// A link that leaves the book: one with a URL scheme (`https:`,
/// `mailto:`), not a place in the book's own files.
pub(crate) fn is_external(target: &str) -> bool {
    let Some((scheme, rest)) = target.split_once(':') else {
        return false;
    };
    let mut letters = scheme.chars();
    letters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && letters.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.'))
        && !rest.is_empty()
}

#[cfg(test)]
mod tests {
    #[test]
    fn only_links_with_a_scheme_leave_the_book() {
        assert!(super::is_external("https://example.com/"));
        assert!(super::is_external("mailto:a@b.c"));
        assert!(!super::is_external("#irc://irc.freenode.net/"));
        assert!(!super::is_external("chapter-2.xhtml#x"));
        assert!(!super::is_external("#note-1"));
    }
}
