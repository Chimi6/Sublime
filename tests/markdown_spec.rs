//! Runs every CommonMark and GFM specification example through the parser
//! and HTML writer. Every example must produce the expected HTML exactly.

use std::path::PathBuf;

use sublime::io::html::push_html;
use sublime::io::json::{JsonTokenizer, Token};
use sublime::io::markdown::{Options, Parser};

struct Example {
    markdown: String,
    html: String,
    number: String,
    section: String,
}

fn fixture(name: &str) -> PathBuf {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("tests/fixtures/markdown");
    path.push(name);
    path
}

/// Reads an array of flat objects with the tokenizer from our own JSON module.
fn load(name: &str) -> Vec<Example> {
    let bytes = std::fs::read(fixture(name)).expect("fixture readable");
    let mut tokenizer = JsonTokenizer::new(&bytes[..]);
    let mut examples = Vec::new();
    tokenizer.expect(Token::BeginArray).unwrap();
    loop {
        let token = tokenizer.next_token().unwrap();
        match token {
            Token::EndArray => break,
            Token::Comma => continue,
            Token::BeginObject => {
                let mut example = Example {
                    markdown: String::new(),
                    html: String::new(),
                    number: String::new(),
                    section: String::new(),
                };
                loop {
                    let key_token = tokenizer.next_token().unwrap();
                    match key_token {
                        Token::EndObject => break,
                        Token::Comma => continue,
                        Token::String => {
                            let key = tokenizer.text().to_string();
                            tokenizer.expect(Token::Colon).unwrap();
                            let value_token = tokenizer.next_token().unwrap();
                            let value = tokenizer.text().to_string();
                            match key.as_str() {
                                "markdown" => example.markdown = value,
                                "html" => example.html = value,
                                "example" => example.number = value,
                                "section" => example.section = value,
                                _ => tokenizer.skip_value(value_token).unwrap(),
                            }
                        }
                        other => panic!("unexpected {other:?}"),
                    }
                }
                examples.push(example);
            }
            other => panic!("unexpected {other:?}"),
        }
    }
    examples
}

fn render(markdown: &str, options: Options) -> String {
    let parser = Parser::new_with_options(markdown, options);
    let mut html = String::new();
    push_html(&mut html, parser);
    html
}

/// `SPEC_ONLY=4,5,7` runs only those example numbers and prints every diff.
fn run_corpus(name: &str, options: Options) {
    let examples = load(name);
    let only: Vec<String> = std::env::var("SPEC_ONLY")
        .map(|list| list.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let mut failures = Vec::new();
    let trace = std::env::var_os("SPEC_TRACE").is_some();
    for example in &examples {
        if !only.is_empty() && !only.contains(&example.number) {
            continue;
        }
        // cmark-gfm marks a few examples as "must not crash" only.
        if example.html.trim() == "<IGNORE>" {
            let _ = render(&example.markdown, options);
            continue;
        }
        // cmark-gfm's task list markup predates the GFM spec's; we follow the spec.
        if name == "cmark-gfm-extensions.json" && example.section == "Task lists" {
            continue;
        }
        if trace {
            eprintln!("example {} ({})", example.number, example.section);
        }
        let actual = render(&example.markdown, options);
        if actual != example.html {
            failures.push(example);
        }
    }
    let total = examples.len();
    let passed = total - failures.len();
    let _ = passed;
    eprintln!("{name}: {passed}/{total} passed");
    let failing: Vec<&str> = failures
        .iter()
        .map(|failure| failure.number.as_str())
        .collect();
    eprintln!("failing: {}", failing.join(","));
    let show = if only.is_empty() { 8 } else { failures.len() };
    for failure in failures.iter().take(show) {
        eprintln!(
            "--- example {} ({})\n{:?}\nexpected:\n{}\nactual:\n{}",
            failure.number,
            failure.section,
            failure.markdown,
            failure.html,
            render(&failure.markdown, options)
        );
    }
    assert!(
        failures.is_empty(),
        "{name}: {} of {total} examples failed",
        failures.len()
    );
}

#[test]
fn commonmark_spec() {
    // The tag filter and autolink literals are GFM extensions that change
    // CommonMark output; the base corpus runs without them, as cmark-gfm does.
    let options = Options {
        tagfilter: false,
        autolinks: false,
        ..Options::default()
    };
    run_corpus("commonmark.json", options);
}

#[test]
fn gfm_extensions_spec() {
    run_corpus("gfm.json", Options::default());
}

#[test]
fn cmark_gfm_extension_examples() {
    run_corpus("cmark-gfm-extensions.json", Options::default());
}

/// Parses every example, writes it back as Markdown, parses that, and
/// requires the same events. HTML output equality is checked too, as the
/// reader's view of "the same document". `ROUNDTRIP_ONLY=4,5` narrows and
/// prints every diff.
fn run_round_trip(name: &str, options: Options) {
    use sublime::io::markdown::push_markdown;
    let examples = load(name);
    let only: Vec<String> = std::env::var("ROUNDTRIP_ONLY")
        .map(|list| list.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let mut failures = Vec::new();
    for example in &examples {
        if !only.is_empty() && !only.contains(&example.number) {
            continue;
        }
        if example.html.trim() == "<IGNORE>" {
            continue;
        }
        let expected = merged_text(Parser::new_with_options(&example.markdown, options));
        let mut written = String::new();
        push_markdown(
            &mut written,
            Parser::new_with_options(&example.markdown, options),
        );
        let actual = merged_text(Parser::new_with_options(&written, options));
        if actual != expected || render(&written, options) != render(&example.markdown, options) {
            failures.push((example, written));
        }
    }
    let total = examples.len();
    eprintln!(
        "{name} round trip: {}/{total} passed",
        total - failures.len()
    );
    let failing: Vec<&str> = failures
        .iter()
        .map(|(failure, _)| failure.number.as_str())
        .collect();
    eprintln!("failing: {}", failing.join(","));
    let show = if only.is_empty() { 6 } else { failures.len() };
    for (failure, written) in failures.iter().take(show) {
        eprintln!(
            "--- example {} ({})\nsource:\n{:?}\nwritten:\n{:?}\nexpected html:\n{}actual html:\n{}",
            failure.number,
            failure.section,
            failure.markdown,
            written,
            render(&failure.markdown, options),
            render(written, options)
        );
    }
    assert!(
        failures.is_empty(),
        "{name}: {} of {total} examples do not round-trip",
        failures.len()
    );
}

/// The parser splits text at escapes and entities; the split is not part
/// of the document, so adjacent text events are joined before comparing.
fn merged_text<'a>(
    events: impl Iterator<Item = sublime::io::markdown::Event<'a>>,
) -> Vec<sublime::io::markdown::Event<'a>> {
    use std::borrow::Cow;
    use sublime::io::markdown::{CodeBlockKind, Event, Tag};
    let mut merged: Vec<Event<'a>> = Vec::new();
    for event in events {
        // Indented code is the same document as fenced code with no info
        // string; the writer fences indented code that follows a list.
        let event = match event {
            Event::Start(Tag::CodeBlock(CodeBlockKind::Fenced(info))) if info.is_empty() => {
                Event::Start(Tag::CodeBlock(CodeBlockKind::Indented))
            }
            other => other,
        };
        if let (Some(Event::Text(previous)), Event::Text(text)) = (merged.last_mut(), &event) {
            let mut joined = std::mem::take(previous).into_owned();
            joined.push_str(text);
            *previous = Cow::Owned(joined);
            continue;
        }
        merged.push(event);
    }
    merged
}

#[test]
fn commonmark_round_trip() {
    let options = Options {
        tagfilter: false,
        autolinks: false,
        ..Options::default()
    };
    run_round_trip("commonmark.json", options);
}

#[test]
fn gfm_round_trip() {
    run_round_trip("gfm.json", Options::default());
    run_round_trip("cmark-gfm-extensions.json", Options::default());
}
