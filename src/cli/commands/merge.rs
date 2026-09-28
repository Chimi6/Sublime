//! `sublime convert a.png b.jpg c.tif scan.pdf`: images merged into one
//! PDF, a page each in the order given. A JPEG goes in as it is; every
//! other image is read into its page as rows. The PDF is written to a
//! `.part` file and renamed into place when every page is in.

use std::fs::{self, File};
use std::io::{BufWriter, Read};
use std::path::{Path, PathBuf};

use crate::cli::CliError;
use crate::cli::ExitCode;
use crate::cli::args::ConvertArgs;
use crate::converter::{ConvertError, ConvertOptions, Input, Location, Tier};
use crate::converters::image::{self, ImageFormat, pair, reader_for};
use crate::event::{CollectingSink, Context, Event, Hop, MultiSink, Sink};
use crate::io::pdf::PdfDocument;
use crate::registry;

pub fn run(args: &ConvertArgs, renderer: &mut dyn Sink) -> Result<ExitCode, CliError> {
    let known = registry::all_formats();
    let Some(output) = args.output.as_ref().map(PathBuf::from) else {
        return Err(CliError::Usage("merging needs an output PDF".to_string()));
    };
    // Every input is an image we read, checked before anything is written.
    let mut inputs: Vec<(PathBuf, &'static str, ImageFormat)> = Vec::new();
    for input in &args.inputs {
        if input == "-" {
            return Err(CliError::Usage(
                "stdin (-) cannot be one of several inputs".to_string(),
            ));
        }
        let path = PathBuf::from(input);
        if !path.is_file() {
            return Err(CliError::Io {
                action: format!("opening '{input}'"),
                error: std::io::Error::from(std::io::ErrorKind::NotFound),
            });
        }
        let format = super::batch::detect_format(&path, &known).map_err(CliError::Format)?;
        let Some(kind) = reader_for(format.id) else {
            return Err(CliError::Usage(format!(
                "'{input}' is {}, not an image: only images merge into a PDF",
                format.display_name
            )));
        };
        inputs.push((path, format.id, kind));
    }
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let options = ConvertOptions {
        strict: args.strict,
        sheet: args.sheet.clone(),
        quality: args.quality,
        page: args.page,
        font: crate::cli::commands::font_option(args.font.as_deref())?,
    };
    let mut collector = CollectingSink::new();
    let written = write_pages(&inputs, &part, &output, &options, renderer, &mut collector);
    if let Err(error) = written {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    fs::rename(&part, &output).map_err(|error| {
        let _ = fs::remove_file(&part);
        CliError::Io {
            action: format!("moving '{}' into place", part.display()),
            error,
        }
    })?;
    if collector.report().is_lossless() {
        Ok(ExitCode::Success)
    } else {
        Ok(ExitCode::Loss)
    }
}

fn write_pages(
    inputs: &[(PathBuf, &'static str, ImageFormat)],
    part: &Path,
    output: &Path,
    options: &ConvertOptions,
    renderer: &mut dyn Sink,
    collector: &mut CollectingSink,
) -> Result<(), CliError> {
    let file = File::create(part).map_err(|error| CliError::Io {
        action: format!("creating '{}'", part.display()),
        error,
    })?;
    let mut writer = BufWriter::new(file);
    let io = |action: String| move |error: std::io::Error| CliError::Io { action, error };
    let mut document =
        PdfDocument::new(&mut writer).map_err(io(format!("writing '{}'", part.display())))?;
    let mut multi = MultiSink::new(vec![renderer, collector]);
    let mut context = Context::new(&mut multi, options);
    for (path, format_id, kind) in inputs {
        let converter = pair(format_id, "pdf");
        context.emit(Event::FileStarted {
            input: path.display().to_string(),
            output: format!("{} (page {})", output.display(), document.page_count() + 1),
        });
        context.emit(Event::PathChosen {
            hops: vec![Hop {
                converter: converter.name,
                from: converter.from.id,
                to: converter.to.id,
                fidelity: converter.fidelity.clone(),
                tier: Tier::Native,
            }],
        });
        let mut file = File::open(path).map_err(io(format!("opening '{}'", path.display())))?;
        if *kind == ImageFormat::Jpeg {
            let mut jpeg = Vec::new();
            file.read_to_end(&mut jpeg)
                .map_err(io(format!("reading '{}'", path.display())))?;
            document.jpeg_page(&jpeg).map_err(|failure| {
                CliError::Convert(ConvertError::Malformed {
                    location: Location::default(),
                    message: format!("{}: {}", path.display(), failure.0),
                })
            })?;
        } else {
            let mut page = document.image_page();
            let mut input = Input::Rewindable(&mut file);
            image::read_image(*kind, &mut input, &mut page, converter.name, &mut context)
                .map_err(CliError::Convert)?;
        }
    }
    document
        .finish()
        .map_err(io(format!("writing '{}'", part.display())))
}
