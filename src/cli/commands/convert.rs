//! `sublime convert`: detect formats, plan, execute, report.

use std::fs::File;
use std::io::{self, BufWriter, Read, Write};
use std::path::Path;

use crate::cli::args::ConvertArgs;
use crate::cli::{CliError, ExitCode};
use crate::converter::{ConvertOptions, Input};
use crate::event::{CollectingSink, Context, Event, MultiSink, Sink};
use crate::format::{self, Format};
use crate::planner::{self, PlanOptions};
use crate::registry;

const SNIFF_SIZE: usize = 64;

pub fn run(
    args: &ConvertArgs,
    renderer: &mut dyn Sink,
    stdout: &mut dyn Write,
) -> Result<ExitCode, CliError> {
    if super::batch::is_batch(args) {
        return super::batch::run(args, renderer);
    }
    let known = registry::all_formats();
    let input_name = args.inputs.first().cloned().unwrap_or_default();
    let reads_stdin = input_name == "-";
    let mut input_file = open_input_file(args, reads_stdin)?;
    let from = resolve_from(args, &known, reads_stdin, input_file.as_mut())?;
    let to = resolve_to(args, &known)?;
    refuse_clobbering_another_format(args, to)?;
    let via = match &args.via {
        Some(id) => Some(format::find_by_id(id, &known)?),
        None => None,
    };

    let plan_options = PlanOptions {
        strict: args.strict,
        via,
    };
    let plan = planner::plan(registry::all_converters(), from, to, &plan_options)?;

    let convert_options = ConvertOptions {
        strict: args.strict,
        sheet: args.sheet.clone(),
    };
    let mut collector = CollectingSink::new();
    let mut multi = MultiSink::new(vec![renderer, &mut collector]);
    let mut context = Context::new(&mut multi, &convert_options);
    context.emit(Event::PathChosen {
        hops: plan.describe(),
    });

    let mut stdin = io::stdin();
    let input = match input_file.as_mut() {
        Some(file) => Input::Rewindable(file),
        None => Input::Stream(&mut stdin),
    };

    let result = match &args.output {
        Some(path) => convert_to_file(&plan, input, Path::new(path), &mut context),
        None => convert_to_stdout(&plan, input, stdout, &mut context),
    };
    result?;

    let report = collector.report();
    if report.is_lossless() {
        Ok(ExitCode::Success)
    } else {
        Ok(ExitCode::Loss)
    }
}

fn open_input_file(args: &ConvertArgs, reads_stdin: bool) -> Result<Option<File>, CliError> {
    if reads_stdin {
        return Ok(None);
    }
    let input_name = args.inputs.first().cloned().unwrap_or_default();
    match File::open(&input_name) {
        Ok(file) => Ok(Some(file)),
        Err(error) => Err(CliError::Io {
            action: format!("opening '{input_name}'"),
            error,
        }),
    }
}

fn resolve_from(
    args: &ConvertArgs,
    known: &[&'static Format],
    reads_stdin: bool,
    input_file: Option<&mut File>,
) -> Result<&'static Format, CliError> {
    if let Some(id) = &args.from {
        let found = format::find_by_id(id, known)?;
        return Ok(found);
    }
    if reads_stdin {
        return Err(CliError::Format(format::FormatError::Undetectable));
    }
    let input_name = args.inputs.first().cloned().unwrap_or_default();
    let by_extension = format::find_by_extension(Path::new(&input_name), known);
    let extension_error = match by_extension {
        Ok(found) => return Ok(found),
        Err(error) => error,
    };
    let file = match input_file {
        Some(file) => file,
        None => return Err(CliError::Format(format::FormatError::Undetectable)),
    };
    let mut head = [0u8; SNIFF_SIZE];
    let read_count = file.read(&mut head).map_err(|error| CliError::Io {
        action: format!("reading '{input_name}'"),
        error,
    })?;
    rewind_file(file, &input_name)?;
    let by_magic = format::find_by_magic(&head[..read_count], known);
    match by_magic {
        Ok(found) => Ok(found),
        Err(_) => Err(CliError::Format(extension_error)),
    }
}

fn rewind_file(file: &mut File, name: &str) -> Result<(), CliError> {
    use crate::converter::RewindableRead;
    let rewound = RewindableRead::rewind(file);
    rewound.map_err(|error| CliError::Io {
        action: format!("rewinding '{name}'"),
        error,
    })
}

/// `a.csv b.csv --to json` reads as two inputs to a person and as
/// "write a.csv's JSON over b.csv" to the parser; when the output exists
/// and its extension names a format other than the target, refuse and
/// point at the batch spelling.
fn refuse_clobbering_another_format(
    args: &ConvertArgs,
    to: &'static Format,
) -> Result<(), CliError> {
    let Some(output) = &args.output else {
        return Ok(());
    };
    let path = Path::new(output);
    if to.extensions.is_empty() || !path.is_file() {
        return Ok(());
    }
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .unwrap_or_default();
    let matches_target = to.extensions.iter().any(|known| *known == extension);
    if matches_target {
        return Ok(());
    }
    Err(CliError::Usage(format!(
        "'{output}' exists and is not a {} file; to convert both files, add --out-dir <dir>",
        to.id
    )))
}

fn resolve_to(args: &ConvertArgs, known: &[&'static Format]) -> Result<&'static Format, CliError> {
    if let Some(id) = &args.to {
        let found = format::find_by_id(id, known)?;
        return Ok(found);
    }
    let output = match &args.output {
        Some(output) => output,
        None => {
            return Err(CliError::Format(format::FormatError::NoExtension(
                "(stdout)".to_string(),
            )));
        }
    };
    let found = format::find_by_extension(Path::new(output), known)?;
    Ok(found)
}

fn convert_to_stdout(
    plan: &planner::Plan,
    input: Input<'_>,
    stdout: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), CliError> {
    let mut writer = BufWriter::new(stdout);
    planner::execute(plan, input, &mut writer, context)?;
    writer.flush().map_err(|error| CliError::Io {
        action: "writing stdout".to_string(),
        error,
    })
}

fn convert_to_file(
    plan: &planner::Plan,
    input: Input<'_>,
    path: &Path,
    context: &mut Context<'_>,
) -> Result<(), CliError> {
    super::batch::write_via_part(plan, input, path, context)
}
