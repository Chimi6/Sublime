//! PDF -> text against pdf-extract (lopdf underneath), the Rust crate for
//! it; poppler's pdftotext is run by the pair script as the C reference.

use std::fs::File;
use std::io::{BufWriter, Write};

use crate::common::run_ours;
use sublime::registry;

pub fn run(mode: &str, args: &[String]) -> Result<(), String> {
    match (mode, args) {
        ("ours-pdf-text", [input, output]) => {
            let converter = registry::all_converters()
                .iter()
                .find(|converter| converter.name() == "pdf-to-text")
                .ok_or("no pdf-to-text converter")?;
            run_ours(*converter, input, output)
        }
        ("crates-pdf-text", [input, output]) => crates_pdf_text(input, output),
        _ => Err("pdf-text modes: ours-pdf-text, crates-pdf-text <in> <out>".to_string()),
    }
}

fn crates_pdf_text(input: &str, output: &str) -> Result<(), String> {
    let bytes = std::fs::read(input).map_err(|error| error.to_string())?;
    let text = pdf_extract::extract_text_from_mem(&bytes).map_err(|error| error.to_string())?;
    let mut file = BufWriter::new(File::create(output).map_err(|error| error.to_string())?);
    file.write_all(text.as_bytes()).map_err(|error| error.to_string())?;
    file.flush().map_err(|error| error.to_string())
}
