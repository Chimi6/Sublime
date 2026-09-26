//! `sublime convert` over many inputs: files, directories, and glob
//! patterns, each converted to a file named after its input with the
//! target's extension, into `--out-dir` or beside the input. Files are
//! converted in parallel, one worker per CPU; each writes to a `.part`
//! file renamed into place when the conversion succeeds, so an output is
//! either whole or absent. One file's failure does not stop the others.
//! Events are collected per file and rendered when that file finishes, in
//! completion order, so the report stays readable under parallelism.

use std::fs::{self, File};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::mpsc;

use crate::cli::args::ConvertArgs;
use crate::cli::{CliError, ExitCode};
use crate::converter::{ConvertOptions, Input};
use crate::event::{CollectingSink, Context, Event, Sink};
use crate::format::{self, Format, FormatError};
use crate::planner::{self, Plan, PlanOptions};
use crate::registry;

const SNIFF_SIZE: usize = 64;

/// One file to convert.
struct Job {
    input: PathBuf,
    output: PathBuf,
    plan: Plan,
}

/// True when the arguments describe a batch rather than one file.
pub fn is_batch(args: &ConvertArgs) -> bool {
    args.inputs.len() > 1
        || args.out_dir.is_some()
        || args.recursive
        || args.dry_run
        || args
            .inputs
            .iter()
            .any(|input| has_glob(input) || Path::new(input).is_dir())
}

pub fn run(args: &ConvertArgs, renderer: &mut dyn Sink) -> Result<ExitCode, CliError> {
    if args.inputs.iter().any(|input| input == "-") {
        return Err(CliError::Usage(
            "stdin (-) cannot be one of several inputs".to_string(),
        ));
    }
    let known = registry::all_formats();
    let to = match &args.to {
        Some(id) => format::find_by_id(id, &known)?,
        None => {
            return Err(CliError::Usage(
                "several inputs need --to <format>".to_string(),
            ));
        }
    };
    let from_override = match &args.from {
        Some(id) => Some(format::find_by_id(id, &known)?),
        None => None,
    };
    let via = match &args.via {
        Some(id) => Some(format::find_by_id(id, &known)?),
        None => None,
    };
    let plan_options = PlanOptions {
        strict: args.strict,
        via,
    };
    let out_dir = args.out_dir.as_ref().map(PathBuf::from);

    // Every input file, with the directory it was found under (for
    // mirroring the structure under --out-dir).
    let mut files: Vec<(PathBuf, Option<PathBuf>)> = Vec::new();
    for input in &args.inputs {
        let path = Path::new(input);
        if path.is_dir() {
            collect_directory(path, args.recursive, &mut files)?;
        } else if !path.exists() && has_glob(input) {
            let matches = expand_glob(input)?;
            if matches.is_empty() {
                return Err(CliError::Usage(format!("no files match '{input}'")));
            }
            for found in matches {
                files.push((found, None));
            }
        } else {
            if !path.exists() {
                return Err(CliError::Io {
                    action: format!("opening '{input}'"),
                    error: std::io::Error::from(std::io::ErrorKind::NotFound),
                });
            }
            files.push((path.to_path_buf(), None));
        }
    }

    let mut jobs: Vec<Job> = Vec::new();
    let mut skipped: u64 = 0;
    for (input, root) in files {
        let from = match from_override {
            Some(from) => from,
            None => match detect_format(&input, &known) {
                Ok(from) => from,
                Err(error) => {
                    // Files inside a directory or glob that we cannot
                    // read are skipped; a file named outright is an error.
                    let named_outright =
                        root.is_none() && !args.inputs.iter().any(|input| has_glob(input));
                    if named_outright {
                        return Err(CliError::Format(error));
                    }
                    skipped += 1;
                    continue;
                }
            },
        };
        let output = output_path(&input, root.as_deref(), out_dir.as_deref(), to);
        if output == input {
            return Err(CliError::Usage(format!(
                "'{}' would overwrite its own input; give --out-dir",
                input.display()
            )));
        }
        let plan = planner::plan(registry::all_converters(), from, to, &plan_options)?;
        jobs.push(Job {
            input,
            output,
            plan,
        });
    }
    check_collisions(&jobs)?;

    if args.dry_run {
        for job in &jobs {
            renderer.emit(&Event::FileStarted {
                input: job.input.display().to_string(),
                output: job.output.display().to_string(),
            });
            renderer.emit(&Event::PathChosen {
                hops: job.plan.describe(),
            });
        }
        renderer.emit(&Event::BatchFinished {
            converted: jobs.len() as u64,
            lossy: 0,
            failed: 0,
            skipped,
            dry_run: true,
        });
        return Ok(ExitCode::Success);
    }

    let options = ConvertOptions {
        strict: args.strict,
        sheet: args.sheet.clone(),
    };
    let workers = args
        .jobs
        .unwrap_or_else(|| {
            std::thread::available_parallelism()
                .map(|count| count.get())
                .unwrap_or(1)
        })
        .min(jobs.len().max(1));
    let (converted, lossy, failed) = convert_all(&jobs, workers, &options, renderer);
    renderer.emit(&Event::BatchFinished {
        converted,
        lossy,
        failed,
        skipped,
        dry_run: false,
    });
    if failed > 0 {
        Ok(ExitCode::Error)
    } else if lossy > 0 {
        Ok(ExitCode::Loss)
    } else {
        Ok(ExitCode::Success)
    }
}

/// What one worker sends back for a file.
struct Outcome {
    job: usize,
    events: Vec<Event>,
    result: Result<bool, CliError>,
}

/// Converts every job on `workers` threads; returns (converted, lossy, failed).
fn convert_all(
    jobs: &[Job],
    workers: usize,
    options: &ConvertOptions,
    renderer: &mut dyn Sink,
) -> (u64, u64, u64) {
    let next = Mutex::new(0usize);
    let (sender, receiver) = mpsc::channel::<Outcome>();
    let mut converted = 0;
    let mut lossy = 0;
    let mut failed = 0;
    std::thread::scope(|scope| {
        for _ in 0..workers {
            let sender = sender.clone();
            let next = &next;
            scope.spawn(move || {
                loop {
                    let index = {
                        let mut cursor = match next.lock() {
                            Ok(cursor) => cursor,
                            Err(_) => return,
                        };
                        let index = *cursor;
                        *cursor += 1;
                        index
                    };
                    let Some(job) = jobs.get(index) else {
                        return;
                    };
                    let mut sink = CollectingSink::new();
                    let result = convert_one(job, options, &mut sink);
                    let events = sink.into_events();
                    if sender
                        .send(Outcome {
                            job: index,
                            events,
                            result,
                        })
                        .is_err()
                    {
                        return;
                    }
                }
            });
        }
        drop(sender);
        for outcome in receiver {
            let job = &jobs[outcome.job];
            renderer.emit(&Event::FileStarted {
                input: job.input.display().to_string(),
                output: job.output.display().to_string(),
            });
            for event in &outcome.events {
                renderer.emit(event);
            }
            match outcome.result {
                Ok(lossless) => {
                    converted += 1;
                    if !lossless {
                        lossy += 1;
                    }
                }
                Err(error) => {
                    failed += 1;
                    renderer.emit(&Event::FileFailed {
                        input: job.input.display().to_string(),
                        message: error.to_string(),
                    });
                }
            }
        }
    });
    (converted, lossy, failed)
}

/// Converts one job into its output through a `.part` file; true when
/// the conversion was lossless.
fn convert_one(
    job: &Job,
    options: &ConvertOptions,
    sink: &mut CollectingSink,
) -> Result<bool, CliError> {
    let mut file = File::open(&job.input).map_err(|error| CliError::Io {
        action: format!("opening '{}'", job.input.display()),
        error,
    })?;
    if let Some(parent) = job.output.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|error| CliError::Io {
                action: format!("creating '{}'", parent.display()),
                error,
            })?;
        }
    }
    {
        let mut context = Context::new(sink, options);
        context.emit(Event::PathChosen {
            hops: job.plan.describe(),
        });
        write_via_part(
            &job.plan,
            Input::Rewindable(&mut file),
            &job.output,
            &mut context,
        )?;
    }
    Ok(sink.report().is_lossless())
}

/// Runs the plan into `<output>.part` and renames it into place, so a
/// failed conversion leaves no half-written output and never disturbs an
/// existing one.
pub fn write_via_part(
    plan: &Plan,
    input: Input<'_>,
    output: &Path,
    context: &mut Context<'_>,
) -> Result<(), CliError> {
    let mut part = output.as_os_str().to_owned();
    part.push(".part");
    let part = PathBuf::from(part);
    let file = File::create(&part).map_err(|error| CliError::Io {
        action: format!("creating '{}'", part.display()),
        error,
    })?;
    let mut writer = BufWriter::new(file);
    let executed = planner::execute(plan, input, &mut writer, context);
    let flushed = match executed {
        Ok(()) => writer.flush().map_err(|error| CliError::Io {
            action: format!("writing '{}'", part.display()),
            error,
        }),
        Err(error) => Err(CliError::Convert(error)),
    };
    drop(writer);
    if let Err(error) = flushed {
        let _ = fs::remove_file(&part);
        return Err(error);
    }
    fs::rename(&part, output).map_err(|error| {
        let _ = fs::remove_file(&part);
        CliError::Io {
            action: format!("moving '{}' into place", part.display()),
            error,
        }
    })
}

/// The output for `input`: its name with the target's extension, under
/// `out_dir` (mirroring the path below `root` when it came from a
/// directory) or beside the input.
fn output_path(input: &Path, root: Option<&Path>, out_dir: Option<&Path>, to: &Format) -> PathBuf {
    let extension = to.extensions.first().copied().unwrap_or(to.id);
    let file_name = input.with_extension(extension);
    let file_name = file_name.file_name().map(PathBuf::from).unwrap_or_default();
    match out_dir {
        Some(out_dir) => {
            let relative_dir = root
                .and_then(|root| {
                    input
                        .parent()
                        .and_then(|parent| parent.strip_prefix(root).ok())
                })
                .map(Path::to_path_buf)
                .unwrap_or_default();
            out_dir.join(relative_dir).join(file_name)
        }
        None => input.with_extension(extension),
    }
}

fn check_collisions(jobs: &[Job]) -> Result<(), CliError> {
    let mut seen: Vec<&Path> = Vec::with_capacity(jobs.len());
    for job in jobs {
        if seen.contains(&job.output.as_path()) {
            return Err(CliError::Usage(format!(
                "two inputs would both write '{}'; give --out-dir or convert them separately",
                job.output.display()
            )));
        }
        seen.push(&job.output);
    }
    Ok(())
}

fn collect_directory(
    dir: &Path,
    recursive: bool,
    files: &mut Vec<(PathBuf, Option<PathBuf>)>,
) -> Result<(), CliError> {
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        let entries = fs::read_dir(&current).map_err(|error| CliError::Io {
            action: format!("listing '{}'", current.display()),
            error,
        })?;
        let mut names: Vec<PathBuf> = entries
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        names.sort();
        for path in names {
            if path.is_dir() {
                if recursive {
                    pending.push(path);
                }
            } else {
                files.push((path, Some(dir.to_path_buf())));
            }
        }
    }
    Ok(())
}

/// The input's format by extension, else by its first bytes.
fn detect_format(path: &Path, known: &[&'static Format]) -> Result<&'static Format, FormatError> {
    let by_extension = format::find_by_extension(path, known);
    let extension_error = match by_extension {
        Ok(found) => return Ok(found),
        Err(error) => error,
    };
    let mut head = [0u8; SNIFF_SIZE];
    let read_count = File::open(path)
        .and_then(|mut file| file.read(&mut head))
        .unwrap_or(0);
    match format::find_by_magic(&head[..read_count], known) {
        Ok(found) => Ok(found),
        Err(_) => Err(extension_error),
    }
}

// ---- globs, for shells that do not expand them (Windows) ----

fn has_glob(text: &str) -> bool {
    text.contains(['*', '?'])
}

/// Files matching a pattern with `*` and `?` inside a path segment and
/// `**` for any depth of directories.
fn expand_glob(pattern: &str) -> Result<Vec<PathBuf>, CliError> {
    let normalized = pattern.replace('\\', "/");
    let segments: Vec<&str> = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect();
    let start = if normalized.starts_with('/') {
        PathBuf::from("/")
    } else {
        PathBuf::from(".")
    };
    let mut matches = Vec::new();
    walk_glob(&start, &segments, &mut matches);
    matches.sort();
    // Paths under "." read better without the prefix.
    let cleaned = matches
        .into_iter()
        .map(|path| {
            path.strip_prefix("./")
                .map(Path::to_path_buf)
                .unwrap_or(path)
        })
        .collect();
    Ok(cleaned)
}

fn walk_glob(dir: &Path, segments: &[&str], matches: &mut Vec<PathBuf>) {
    let Some((segment, rest)) = segments.split_first() else {
        return;
    };
    if *segment == "**" {
        walk_glob(dir, rest, matches);
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk_glob(&path, segments, matches);
                }
            }
        }
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !segment_matches(segment, name) {
            continue;
        }
        let path = entry.path();
        if rest.is_empty() {
            if path.is_file() {
                matches.push(path);
            }
        } else if path.is_dir() {
            walk_glob(&path, rest, matches);
        }
    }
}

/// `*` matches any run, `?` one character, within one name.
fn segment_matches(pattern: &str, name: &str) -> bool {
    let pattern: Vec<char> = pattern.chars().collect();
    let name: Vec<char> = name.chars().collect();
    let (mut p, mut n) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if p < pattern.len() && (pattern[p] == '?' || pattern[p] == name[n]) {
            p += 1;
            n += 1;
        } else if p < pattern.len() && pattern[p] == '*' {
            star = Some((p, n));
            p += 1;
        } else if let Some((star_p, star_n)) = star {
            p = star_p + 1;
            n = star_n + 1;
            star = Some((star_p, star_n + 1));
        } else {
            return false;
        }
    }
    while p < pattern.len() && pattern[p] == '*' {
        p += 1;
    }
    p == pattern.len()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_patterns() {
        assert!(segment_matches("*.csv", "a.csv"));
        assert!(segment_matches("a?.csv", "ab.csv"));
        assert!(!segment_matches("a?.csv", "abc.csv"));
        assert!(segment_matches("*", "anything"));
        assert!(segment_matches("*a*b*", "xaxbx"));
        assert!(!segment_matches("*.csv", "a.tsv"));
    }

    #[test]
    fn output_paths_follow_the_rules() {
        let to = &crate::format::formats::JSON;
        assert_eq!(
            output_path(Path::new("dir/a.csv"), None, None, to),
            PathBuf::from("dir/a.json")
        );
        assert_eq!(
            output_path(Path::new("dir/a.csv"), None, Some(Path::new("out")), to),
            PathBuf::from("out/a.json")
        );
        assert_eq!(
            output_path(
                Path::new("docs/sub/a.csv"),
                Some(Path::new("docs")),
                Some(Path::new("out")),
                to
            ),
            PathBuf::from("out/sub/a.json")
        );
    }
}
