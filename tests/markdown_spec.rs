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
