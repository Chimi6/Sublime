//! Reads plain text into the Markdown event stream: a paragraph per run
//! of non-blank lines, the lines inside a paragraph joined by soft
//! breaks. Nothing in the text is markup; the writers escape whatever
//! would read as Markdown, so the text comes out as it went in.

use std::borrow::Cow;

use crate::io::markdown::{Event, EventSink, Tag, TagEnd};

/// Parses `text` and pushes every event into `sink`.
pub fn parse_into<'a>(text: &'a str, sink: &mut dyn EventSink<'a>) {
    let mut open = false;
    for line in text.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            if open {
                sink.event(Event::End(TagEnd::Paragraph));
                open = false;
            }
            continue;
        }
        if open {
            sink.event(Event::SoftBreak);
        } else {
            sink.event(Event::Start(Tag::Paragraph));
            open = true;
        }
        sink.event(Event::Text(Cow::Borrowed(line)));
    }
    if open {
        sink.event(Event::End(TagEnd::Paragraph));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paragraphs_from_blank_lines() {
        let mut events: Vec<Event<'_>> = Vec::new();
        parse_into("one\ntwo\n\n\nthree\r\n", &mut events);
        assert_eq!(
            events,
            vec![
                Event::Start(Tag::Paragraph),
                Event::Text(Cow::Borrowed("one")),
                Event::SoftBreak,
                Event::Text(Cow::Borrowed("two")),
                Event::End(TagEnd::Paragraph),
                Event::Start(Tag::Paragraph),
                Event::Text(Cow::Borrowed("three")),
                Event::End(TagEnd::Paragraph),
            ]
        );
    }

    #[test]
    fn empty_text_is_no_events() {
        let mut events: Vec<Event<'_>> = Vec::new();
        parse_into("\n  \n", &mut events);
        assert!(events.is_empty());
    }
}
