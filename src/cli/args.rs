//! Hand-written argument parser. Five commands, a handful of flags.

use std::fmt;
use std::path::Path;

pub const HELP: &str = "\
sublime: universal efficient file conversion

USAGE
  sublime convert <input> [output] [--to <format>] [--from <format>] [--strict] [--via <format>] [--sheet <name|number>] [--quality <1-100>]
  sublime convert <inputs...> [out-dir/] --to <format> [--out-dir <dir>] [-r] [--jobs <n>] [--dry-run]
  sublime check <from> <to> [--strict]
  sublime formats
  sublime paths [--markdown]
  sublime version

COMMANDS
  convert   Convert a file. Formats come from extensions unless --from/--to are given.
            Use '-' as input to read stdin. Omit output to write stdout.
  check     Show the path and fidelity between two formats without converting.
  formats   List every known format.
  paths     List every conversion path. --markdown emits DOCS/FORMATS.md.
  inspect   Dump an iWork package's object graph (dev-tools builds only).
  version   Print the version.

FLAGS
  --strict            Refuse any path that is lossy or conditional.
  --sheet <name|n>    The worksheet to read from a workbook (a name or a 1-based number; the first when absent), or the name to give the sheet written.
  --quality <1-100>   The quality a lossy image (JPEG) is written at; 85 when absent.
  --out-dir <dir>     Batch: write outputs into this directory (created if needed), keeping each input's name with the new extension. A trailing positional ending in / does the same. Without it, outputs go beside their inputs.
  -r, --recursive     Batch: descend into directories given as inputs, mirroring their structure under --out-dir.
  --jobs <n>          Batch: files converted at once (default: the CPU count).
  --dry-run           Batch: list what would be written and how, without converting.
  --via <format>      Force the path through a format.
  -q                  Errors only.
  -v                  Steps and timings.
  -vv                 Debug and progress.
  --log-format <fmt>  human (default) or json (one event per line on stderr).
  --no-color          Never use ANSI color.
  -h, --help          This text.

ENVIRONMENT
  SUBLIME_LOG=quiet|normal|verbose|debug   Same as the verbosity flags.
  NO_COLOR                                 Disables color when set.

EXIT CODES
  0 success (lossless)   1 error   2 completed with loss   3 no path   4 bad usage
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
    Debug,
}

impl Verbosity {
    pub fn from_env_value(value: &str) -> Option<Verbosity> {
        let lowered = value.to_ascii_lowercase();
        match lowered.as_str() {
            "quiet" => Some(Verbosity::Quiet),
            "normal" => Some(Verbosity::Normal),
            "verbose" => Some(Verbosity::Verbose),
            "debug" => Some(Verbosity::Debug),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    Human,
    Json,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GlobalArgs {
    pub verbosity: Option<Verbosity>,
    pub log_format: LogFormat,
    pub no_color: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConvertArgs {
    /// Files, directories, or glob patterns; one file may be `-` for stdin.
    pub inputs: Vec<String>,
    /// The output file, when exactly one input converts to one file.
    pub output: Option<String>,
    /// The directory batch outputs go into; beside their inputs when absent.
    pub out_dir: Option<String>,
    pub recursive: bool,
    pub jobs: Option<usize>,
    pub dry_run: bool,
    pub to: Option<String>,
    pub from: Option<String>,
    pub strict: bool,
    pub sheet: Option<String>,
    pub quality: Option<u8>,
    pub via: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InspectArgs {
    pub path: String,
    pub stream: Option<String>,
    pub object: Option<u64>,
    pub message_type: Option<u32>,
    pub depth: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CheckArgs {
    pub from: String,
    pub to: String,
    pub strict: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Convert(ConvertArgs),
    Check(CheckArgs),
    Formats,
    Paths {
        markdown: bool,
    },
    /// `inspect <package> [--stream name] [--object id] [--type id] [--depth n]`;
    /// only does anything in a `dev-tools` build.
    Inspect(InspectArgs),
    Version,
    Help,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub global: GlobalArgs,
    pub command: Command,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgsError {
    MissingCommand,
    UnknownCommand(String),
    UnknownFlag(String),
    MissingValue(String),
    InvalidValue { flag: String, value: String },
    MissingPositional(&'static str),
    TooManyPositionals(String),
}

impl fmt::Display for ArgsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArgsError::MissingCommand => {
                write!(formatter, "no command given; try 'sublime --help'")
            }
            ArgsError::UnknownCommand(name) => write!(formatter, "unknown command '{name}'"),
            ArgsError::UnknownFlag(flag) => write!(formatter, "unknown flag '{flag}'"),
            ArgsError::MissingValue(flag) => write!(formatter, "'{flag}' needs a value"),
            ArgsError::InvalidValue { flag, value } => {
                write!(
                    formatter,
                    "'{value}' is not a valid value for '{flag}'; expected human or json"
                )
            }
            ArgsError::MissingPositional(name) => {
                write!(formatter, "missing required argument <{name}>")
            }
            ArgsError::TooManyPositionals(extra) => {
                write!(formatter, "unexpected argument '{extra}'")
            }
        }
    }
}

impl std::error::Error for ArgsError {}

/// Flags that apply to every command. Extracted first so they may appear anywhere.
struct Extracted {
    global: GlobalArgs,
    remaining: Vec<String>,
    wants_help: bool,
    wants_version: bool,
}

pub fn parse<I: IntoIterator<Item = String>>(args: I) -> Result<ParsedArgs, ArgsError> {
    let extracted = extract_global(args)?;
    if extracted.wants_help {
        return Ok(ParsedArgs {
            global: extracted.global,
            command: Command::Help,
        });
    }
    if extracted.wants_version {
        return Ok(ParsedArgs {
            global: extracted.global,
            command: Command::Version,
        });
    }
    let mut remaining = extracted.remaining.into_iter();
    let command_name = match remaining.next() {
        Some(name) => name,
        None => return Err(ArgsError::MissingCommand),
    };
    let rest: Vec<String> = remaining.collect();
    let command = match command_name.as_str() {
        "convert" => Command::Convert(parse_convert(rest)?),
        "check" => Command::Check(parse_check(rest)?),
        "formats" => {
            reject_extra(rest)?;
            Command::Formats
        }
        "paths" => parse_paths(rest)?,
        "inspect" => Command::Inspect(parse_inspect(rest)?),
        "version" => {
            reject_extra(rest)?;
            Command::Version
        }
        "help" => Command::Help,
        other => return Err(ArgsError::UnknownCommand(other.to_string())),
    };
    Ok(ParsedArgs {
        global: extracted.global,
        command,
    })
}

fn extract_global<I: IntoIterator<Item = String>>(args: I) -> Result<Extracted, ArgsError> {
    let mut global = GlobalArgs {
        verbosity: None,
        log_format: LogFormat::Human,
        no_color: false,
    };
    let mut remaining = Vec::new();
    let mut wants_help = false;
    let mut wants_version = false;
    let mut iterator = args.into_iter();
    while let Some(arg) = iterator.next() {
        match arg.as_str() {
            "-q" | "--quiet" => global.verbosity = Some(Verbosity::Quiet),
            "-v" | "--verbose" => global.verbosity = Some(Verbosity::Verbose),
            "-vv" | "--debug" => global.verbosity = Some(Verbosity::Debug),
            "--no-color" => global.no_color = true,
            "--log-format" => {
                let value = match iterator.next() {
                    Some(value) => value,
                    None => return Err(ArgsError::MissingValue("--log-format".to_string())),
                };
                global.log_format = match value.as_str() {
                    "human" => LogFormat::Human,
                    "json" => LogFormat::Json,
                    _ => {
                        return Err(ArgsError::InvalidValue {
                            flag: "--log-format".to_string(),
                            value,
                        });
                    }
                };
            }
            "-h" | "--help" => wants_help = true,
            "--version" => wants_version = true,
            _ => remaining.push(arg),
        }
    }
    Ok(Extracted {
        global,
        remaining,
        wants_help,
        wants_version,
    })
}

fn parse_convert(rest: Vec<String>) -> Result<ConvertArgs, ArgsError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut to: Option<String> = None;
    let mut from: Option<String> = None;
    let mut via: Option<String> = None;
    let mut sheet: Option<String> = None;
    let mut quality: Option<u8> = None;
    let mut out_dir: Option<String> = None;
    let mut jobs: Option<usize> = None;
    let mut recursive = false;
    let mut dry_run = false;
    let mut strict = false;
    let mut iterator = rest.into_iter();
    while let Some(arg) = iterator.next() {
        match arg.as_str() {
            "--to" => to = Some(take_value(&mut iterator, "--to")?),
            "--from" => from = Some(take_value(&mut iterator, "--from")?),
            "--via" => via = Some(take_value(&mut iterator, "--via")?),
            "--sheet" => sheet = Some(take_value(&mut iterator, "--sheet")?),
            "--quality" => {
                let value = take_value(&mut iterator, "--quality")?;
                quality = Some(match value.parse::<u8>() {
                    Ok(number) if (1..=100).contains(&number) => number,
                    _ => {
                        return Err(ArgsError::InvalidValue {
                            flag: "--quality".to_string(),
                            value,
                        });
                    }
                });
            }
            "--out-dir" => out_dir = Some(take_value(&mut iterator, "--out-dir")?),
            "--jobs" => {
                let value = take_value(&mut iterator, "--jobs")?;
                let count = value.parse::<usize>().ok().filter(|count| *count > 0);
                match count {
                    Some(count) => jobs = Some(count),
                    None => {
                        return Err(ArgsError::InvalidValue {
                            flag: "--jobs".to_string(),
                            value,
                        });
                    }
                }
            }
            "-r" | "--recursive" => recursive = true,
            "--dry-run" => dry_run = true,
            "--strict" => strict = true,
            other if is_flag(other) => return Err(ArgsError::UnknownFlag(other.to_string())),
            _ => positionals.push(arg),
        }
    }
    if positionals.is_empty() {
        return Err(ArgsError::MissingPositional("input"));
    }
    // `a.csv b.csv out/`: a trailing directory is the output directory.
    let trailing_is_directory = positionals.len() >= 2
        && positionals.last().is_some_and(|last| {
            last.ends_with('/') || last.ends_with('\\') || Path::new(last).is_dir()
        });
    let mut output = None;
    if trailing_is_directory {
        let last = positionals.pop().unwrap_or_default();
        if out_dir.is_none() {
            out_dir = Some(last);
        }
    } else if positionals.len() == 2 && out_dir.is_none() {
        output = positionals.pop();
    }
    Ok(ConvertArgs {
        inputs: positionals,
        output,
        out_dir,
        recursive,
        jobs,
        dry_run,
        to,
        from,
        strict,
        via,
        sheet,
        quality,
    })
}

fn parse_check(rest: Vec<String>) -> Result<CheckArgs, ArgsError> {
    let mut positionals: Vec<String> = Vec::new();
    let mut strict = false;
    for arg in rest {
        match arg.as_str() {
            "--strict" => strict = true,
            other if is_flag(other) => return Err(ArgsError::UnknownFlag(other.to_string())),
            _ => positionals.push(arg),
        }
    }
    if positionals.len() > 2 {
        return Err(ArgsError::TooManyPositionals(positionals[2].clone()));
    }
    let mut positionals = positionals.into_iter();
    let from = match positionals.next() {
        Some(from) => from,
        None => return Err(ArgsError::MissingPositional("from")),
    };
    let to = match positionals.next() {
        Some(to) => to,
        None => return Err(ArgsError::MissingPositional("to")),
    };
    Ok(CheckArgs { from, to, strict })
}

fn parse_inspect(rest: Vec<String>) -> Result<InspectArgs, ArgsError> {
    let mut path = None;
    let mut stream = None;
    let mut object = None;
    let mut message_type = None;
    let mut depth = 6usize;
    let mut args = rest.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--stream" => stream = Some(flag_value(&mut args, "--stream")?),
            "--object" => {
                let value = flag_value(&mut args, "--object")?;
                object = Some(
                    value
                        .parse()
                        .map_err(|_| ArgsError::UnknownFlag(format!("--object {value}")))?,
                );
            }
            "--type" => {
                let value = flag_value(&mut args, "--type")?;
                message_type = Some(
                    value
                        .parse()
                        .map_err(|_| ArgsError::UnknownFlag(format!("--type {value}")))?,
                );
            }
            "--depth" => {
                let value = flag_value(&mut args, "--depth")?;
                depth = value
                    .parse()
                    .map_err(|_| ArgsError::UnknownFlag(format!("--depth {value}")))?;
            }
            other if is_flag(other) => return Err(ArgsError::UnknownFlag(other.to_string())),
            other if path.is_none() => path = Some(other.to_string()),
            other => return Err(ArgsError::TooManyPositionals(other.to_string())),
        }
    }
    let path = path.ok_or(ArgsError::MissingPositional("package"))?;
    Ok(InspectArgs {
        path,
        stream,
        object,
        message_type,
        depth,
    })
}

fn flag_value(
    args: &mut impl Iterator<Item = String>,
    flag: &'static str,
) -> Result<String, ArgsError> {
    args.next()
        .ok_or_else(|| ArgsError::MissingValue(flag.to_string()))
}

fn parse_paths(rest: Vec<String>) -> Result<Command, ArgsError> {
    let mut markdown = false;
    for arg in rest {
        match arg.as_str() {
            "--markdown" => markdown = true,
            other if is_flag(other) => return Err(ArgsError::UnknownFlag(other.to_string())),
            other => return Err(ArgsError::TooManyPositionals(other.to_string())),
        }
    }
    Ok(Command::Paths { markdown })
}

fn reject_extra(rest: Vec<String>) -> Result<(), ArgsError> {
    if let Some(arg) = rest.into_iter().next() {
        if is_flag(&arg) {
            return Err(ArgsError::UnknownFlag(arg));
        }
        return Err(ArgsError::TooManyPositionals(arg));
    }
    Ok(())
}

fn take_value(
    iterator: &mut impl Iterator<Item = String>,
    flag: &str,
) -> Result<String, ArgsError> {
    match iterator.next() {
        Some(value) => Ok(value),
        None => Err(ArgsError::MissingValue(flag.to_string())),
    }
}

fn is_flag(arg: &str) -> bool {
    arg.starts_with('-') && arg != "-"
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_strs(args: &[&str]) -> Result<ParsedArgs, ArgsError> {
        parse(args.iter().map(|arg| arg.to_string()))
    }

    #[test]
    fn parses_convert_with_all_flags() {
        let parsed = parse_strs(&[
            "convert", "in.csv", "out.json", "--to", "json", "--from", "csv", "--strict", "--via",
            "json", "-v",
        ])
        .unwrap();
        match parsed.command {
            Command::Convert(args) => {
                assert_eq!(args.inputs, vec!["in.csv".to_string()]);
                assert_eq!(args.output.as_deref(), Some("out.json"));
                assert_eq!(args.to.as_deref(), Some("json"));
                assert_eq!(args.from.as_deref(), Some("csv"));
                assert!(args.strict);
                assert_eq!(args.via.as_deref(), Some("json"));
            }
            other => panic!("wrong command {other:?}"),
        }
        assert_eq!(parsed.global.verbosity, Some(Verbosity::Verbose));
    }

    #[test]
    fn convert_output_is_optional() {
        let parsed = parse_strs(&["convert", "-", "--to", "json"]).unwrap();
        match parsed.command {
            Command::Convert(args) => {
                assert_eq!(args.inputs, vec!["-".to_string()]);
                assert!(args.output.is_none());
            }
            other => panic!("wrong command {other:?}"),
        }
    }

    #[test]
    fn convert_requires_input() {
        let error = parse_strs(&["convert"]).unwrap_err();
        assert_eq!(error, ArgsError::MissingPositional("input"));
    }

    #[test]
    fn convert_takes_several_inputs_and_a_trailing_directory() {
        let parsed = parse_strs(&["convert", "a.csv", "b.csv", "c.csv", "--to", "json"]).unwrap();
        match parsed.command {
            Command::Convert(args) => {
                assert_eq!(args.inputs.len(), 3);
                assert!(args.output.is_none());
                assert!(args.out_dir.is_none());
            }
            other => panic!("unexpected {other:?}"),
        }
        let parsed = parse_strs(&[
            "convert", "a.csv", "b.csv", "out/", "--to", "json", "-r", "--jobs", "2",
        ])
        .unwrap();
        match parsed.command {
            Command::Convert(args) => {
                assert_eq!(args.inputs.len(), 2);
                assert_eq!(args.out_dir.as_deref(), Some("out/"));
                assert!(args.recursive);
                assert_eq!(args.jobs, Some(2));
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn parses_check() {
        let parsed = parse_strs(&["check", "csv", "json", "--strict"]).unwrap();
        match parsed.command {
            Command::Check(args) => {
                assert_eq!(args.from, "csv");
                assert_eq!(args.to, "json");
                assert!(args.strict);
            }
            other => panic!("wrong command {other:?}"),
        }
    }

    #[test]
    fn parses_simple_commands() {
        assert!(matches!(
            parse_strs(&["formats"]).unwrap().command,
            Command::Formats
        ));
        assert!(matches!(
            parse_strs(&["paths"]).unwrap().command,
            Command::Paths { markdown: false }
        ));
        assert!(matches!(
            parse_strs(&["paths", "--markdown"]).unwrap().command,
            Command::Paths { markdown: true }
        ));
        assert!(matches!(
            parse_strs(&["version"]).unwrap().command,
            Command::Version
        ));
        assert!(matches!(
            parse_strs(&["--version"]).unwrap().command,
            Command::Version
        ));
        assert!(matches!(
            parse_strs(&["help"]).unwrap().command,
            Command::Help
        ));
        assert!(matches!(
            parse_strs(&["-h"]).unwrap().command,
            Command::Help
        ));
        assert!(matches!(
            parse_strs(&["formats", "--help"]).unwrap().command,
            Command::Help
        ));
    }

    #[test]
    fn global_flags_anywhere() {
        let parsed = parse_strs(&["-q", "formats", "--log-format", "json", "--no-color"]).unwrap();
        assert_eq!(parsed.global.verbosity, Some(Verbosity::Quiet));
        assert_eq!(parsed.global.log_format, LogFormat::Json);
        assert!(parsed.global.no_color);
        let debug = parse_strs(&["-vv", "formats"]).unwrap();
        assert_eq!(debug.global.verbosity, Some(Verbosity::Debug));
    }

    #[test]
    fn missing_flag_value_is_an_error() {
        let error = parse_strs(&["convert", "a", "--to"]).unwrap_err();
        assert_eq!(error, ArgsError::MissingValue("--to".to_string()));
    }

    #[test]
    fn unknown_flag_and_command_are_errors() {
        assert_eq!(
            parse_strs(&["formats", "--bogus"]).unwrap_err(),
            ArgsError::UnknownFlag("--bogus".to_string())
        );
        assert_eq!(
            parse_strs(&["bogus"]).unwrap_err(),
            ArgsError::UnknownCommand("bogus".to_string())
        );
        assert_eq!(parse_strs(&[]).unwrap_err(), ArgsError::MissingCommand);
    }

    #[test]
    fn invalid_log_format_value_is_an_error() {
        let error = parse_strs(&["--log-format", "bogus", "formats"]).unwrap_err();
        assert_eq!(
            error,
            ArgsError::InvalidValue {
                flag: "--log-format".to_string(),
                value: "bogus".to_string()
            }
        );
    }

    #[test]
    fn zero_argument_commands_reject_positionals() {
        assert_eq!(
            parse_strs(&["formats", "extra"]).unwrap_err(),
            ArgsError::TooManyPositionals("extra".to_string())
        );
        assert_eq!(
            parse_strs(&["version", "extra"]).unwrap_err(),
            ArgsError::TooManyPositionals("extra".to_string())
        );
        assert_eq!(
            parse_strs(&["paths", "extra"]).unwrap_err(),
            ArgsError::TooManyPositionals("extra".to_string())
        );
        assert_eq!(
            parse_strs(&["paths", "--bogus"]).unwrap_err(),
            ArgsError::UnknownFlag("--bogus".to_string())
        );
    }

    #[test]
    fn env_values_map_to_verbosity() {
        assert_eq!(Verbosity::from_env_value("quiet"), Some(Verbosity::Quiet));
        assert_eq!(Verbosity::from_env_value("DEBUG"), Some(Verbosity::Debug));
        assert_eq!(Verbosity::from_env_value("loud"), None);
    }
}
