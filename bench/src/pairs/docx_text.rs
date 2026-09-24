//! Word -> text: our path, and `docx-rs` as the reference reader with the
//! paragraph texts joined by newlines (the same output shape).

use std::fs::File;
use std::io::{BufWriter, Write};

use docx_rs::{DocumentChild, ParagraphChild, RunChild, read_docx};
use sublime::converters::docx_to_text::DocxToText;

use crate::common::run_ours;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours", [input, output]) => run_ours(&DocxToText, input, output),
        ("crates", [input, output]) => crates_docx_to_text(input, output),
        _ => Err("docx-text modes: ours <in> <out> | crates <in> <out>".to_string()),
    }
}

fn crates_docx_to_text(input: &str, output: &str) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|error| error.to_string())?;
    let docx = read_docx(&bytes).map_err(|error| format!("{error:?}"))?;
    let file = File::create(output).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for child in &docx.document.children {
        let DocumentChild::Paragraph(paragraph) = child else {
            continue;
        };
        for run in &paragraph.children {
            let ParagraphChild::Run(run) = run else {
                continue;
            };
            for piece in &run.children {
                if let RunChild::Text(text) = piece {
                    writer
                        .write_all(text.text.as_bytes())
                        .map_err(|error| error.to_string())?;
                }
            }
        }
        writer.write_all(b"\n").map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())
}
