//! Markdown -> HTML: a synthetic document generator, our pipeline, and the
//! `pulldown-cmark` reference with the matching extensions enabled.

use std::fs::File;
use std::io::{BufWriter, Write};

use sublime::converters::markdown_to_html::MarkdownToHtml;

use crate::common::{parse_rows, run_ours};

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("gen", [units, path]) => generate(units, path, false),
        ("gen-plain", [units, path]) => generate(units, path, true),
        ("gen-prose", [units, path]) => generate_prose(units, path),
        ("ours", [input, output]) => run_ours(&MarkdownToHtml, input, output),
        ("crates", [input, output]) => crates_markdown_to_html(input, output),
        ("phases", [input]) => phases(input),
        ("loop", [input, seconds]) => parse_loop(input, seconds),
        _ => Err(
            "markdown-html modes: gen <units> <path> | ours <in> <out> | crates <in> <out>"
                .to_string(),
        ),
    }
}

/// Writes `units` paragraphs of prose, which is what most real documents
/// are: long paragraphs of plain sentences with occasional emphasis, a
/// link, or inline code; a heading every eighth paragraph and a short list
/// after every fifth. No tables, code blocks, or footnotes. Each unit is
/// about 0.6 KB.
fn generate_prose(units: &str, path: &str) -> Result<(), String> {
    let count = parse_rows(units)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for index in 0..count {
        if index % 8 == 0 {
            writeln!(writer, "## Heading {index}\n").map_err(|error| error.to_string())?;
        }
        let paragraph = format!(
            "The quick brown fox jumps over the lazy dog while observer {index} takes careful notes about the weather, the light, and the sound of distant traffic.\n\
             Sentences like this one carry *some emphasis* now and then, a [link to somewhere](https://example.com/page/{index}) every so often, and the odd `identifier` in code.\n\
             Most lines, though, are plain words: nothing to escape, nothing to resolve, just text that has to be scanned once and copied to the output.\n\
             A fourth line keeps the paragraph long enough that line joining matters, and a final one ends it with an ordinary full stop.\n\n"
        );
        writer
            .write_all(paragraph.as_bytes())
            .map_err(|error| error.to_string())?;
        if index % 5 == 4 {
            writer
                .write_all(b"- a short list item\n- another short list item\n- and a third one\n\n")
                .map_err(|error| error.to_string())?;
        }
    }
    writer.flush().map_err(|error| error.to_string())
}

/// Writes `units` repetitions of a fixed mix of constructs: headings,
/// paragraphs with inline markup, lists, code, quotes, links, and a table.
/// Each unit is about 1.1 KB. Footnotes and reference definitions appear in
/// one unit out of twenty, as in a real document; a definition per unit
/// would make any implementation's footnote bookkeeping the whole cost.
/// `plain` omits them entirely, for a comparison that no footnote
/// implementation can dominate.
fn generate(units: &str, path: &str, plain: bool) -> Result<(), String> {
    let count = parse_rows(units)?;
    let file = File::create(path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for index in 0..count {
        let has_definitions = !plain && index % 20 == 0;
        let footnote_text = if has_definitions {
            format!(
                "Some text with a footnote reference[^note{index}] and ~~strikethrough~~ words."
            )
        } else {
            "Some text without a footnote and ~~strikethrough~~ words.".to_string()
        };
        let reference_item = if has_definitions {
            format!("- second item with a [reference link][ref{index}]")
        } else {
            format!("- second item with an [inline link](https://example.com/item/{index})")
        };
        let definitions = if has_definitions {
            format!(
                "[ref{index}]: https://example.com/reference/{index}\n[^note{index}]: The footnote text for unit {index}.\n\n"
            )
        } else {
            String::new()
        };
        let nested_item = "  - nested item";
        let unit = format!(
            "## Section {index}\n\n\
             This paragraph has *emphasis*, **strong text**, `inline code`, and a [link](https://example.com/{index} \"Title\").\n\
             It continues on a second line with an autolink <https://example.org/{index}> and an entity &amp; here.\n\
             {footnote_text}\n\n\
             - first item with *emphasis*\n\
             {reference_item}\n\
             {nested_item}\n\
             - [x] a completed task\n\n\
             1. ordered one\n\
             2. ordered two\n\n\
             > A quoted paragraph that spans\n\
             > two lines of quotation.\n\n\
             ```rust\n\
             fn item_{index}() -> u64 {{\n    {index}\n}}\n\
             ```\n\n\
             | Column A | Column B | Column C |\n\
             |:---------|:--------:|---------:|\n\
             | a{index} | b{index} | c{index} |\n\
             | left     | center   | right    |\n\n\
             Visit www.example.com/path{index} or https://example.net/{index}.\n\n\
             {definitions}\
             ---\n\n"
        );
        writer
            .write_all(unit.as_bytes())
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}

/// Parses and renders the file over and over for about `seconds`, as a
/// steady workload for a sampling profiler.
fn parse_loop(input: &str, seconds: &str) -> Result<(), String> {
    use std::time::Instant;
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let budget = seconds.parse::<f64>().map_err(|_| "bad seconds".to_string())?;
    let started = Instant::now();
    let mut iterations = 0u64;
    let mut html: Vec<u8> = Vec::with_capacity(text.len() * 2);
    while started.elapsed().as_secs_f64() < budget {
        html.clear();
        render_streaming(&text, &mut html)?;
        iterations += 1;
    }
    println!("{iterations} iterations, {} bytes last output", html.len());
    Ok(())
}

/// The production path: parser pushing straight into a streaming writer.
fn render_streaming(text: &str, html: &mut Vec<u8>) -> Result<(), String> {
    use sublime::io::html::HtmlWriter;
    use sublime::io::markdown::{Options, parse_into};
    let mut writer = HtmlWriter::streaming(html);
    parse_into(text, Options::default(), &mut writer);
    writer.finish().map_err(|error| error.to_string())
}

/// Times our pipeline in pieces: block structure alone (parser
/// construction), block plus inline (draining events), and the full
/// render through the production path. Each is the median of three runs.
fn phases(input: &str) -> Result<(), String> {
    use std::time::Instant;
    use sublime::io::markdown::Parser;
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let megabytes = text.len() as f64 / 1_048_576.0;
    let mut block_times = Vec::new();
    let mut inline_times = Vec::new();
    let mut render_times = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let parser = Parser::new(&text);
        block_times.push(started.elapsed().as_secs_f64());
        let started = Instant::now();
        let mut count = 0usize;
        for _event in parser {
            count += 1;
        }
        inline_times.push(started.elapsed().as_secs_f64());
        let started = Instant::now();
        let mut html: Vec<u8> = Vec::with_capacity(text.len() * 2);
        render_streaming(&text, &mut html)?;
        render_times.push(started.elapsed().as_secs_f64());
        let _ = (count, html.len());
    }
    let median = |times: &mut Vec<f64>| {
        times.sort_by(|left, right| left.total_cmp(right));
        times[1]
    };
    let block = median(&mut block_times);
    let with_inline = median(&mut inline_times);
    let full = median(&mut render_times);
    println!("input: {megabytes:.1} MB");
    println!(
        "block structure only:   {:.3} s  ({:.1} MB/s)",
        block,
        megabytes / block
    );
    println!(
        "block + inline (events): {:.3} s  ({:.1} MB/s)  inline share {:.3} s",
        block + with_inline,
        megabytes / (block + with_inline),
        with_inline
    );
    println!(
        "full parse + render:     {:.3} s  ({:.1} MB/s)  render share {:.3} s",
        full,
        megabytes / full,
        full - block - with_inline
    );
    let mut reference_times = Vec::new();
    for _ in 0..3 {
        let started = Instant::now();
        let html = reference_html(&text);
        reference_times.push(started.elapsed().as_secs_f64());
        let _ = html.len();
    }
    let reference = median(&mut reference_times);
    println!(
        "reference parse + render: {:.3} s  ({:.1} MB/s)",
        reference,
        megabytes / reference
    );
    Ok(())
}

/// Reference pipeline: `pulldown-cmark` with tables, footnotes,
/// strikethrough, task lists, and GFM autolinks, rendered by its HTML writer.
fn reference_html(text: &str) -> String {
    use pulldown_cmark::{Options, Parser, html};
    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);
    options.insert(Options::ENABLE_GFM);
    let parser = Parser::new_ext(text, options);
    let mut rendered = String::with_capacity(text.len() + text.len() / 2);
    html::push_html(&mut rendered, parser);
    rendered
}

fn crates_markdown_to_html(input: &str, output: &str) -> Result<(), String> {
    let text = std::fs::read_to_string(input).map_err(|error| error.to_string())?;
    let rendered = reference_html(&text);
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    writer
        .write_all(rendered.as_bytes())
        .map_err(|error| error.to_string())?;
    writer.flush().map_err(|error| error.to_string())
}
