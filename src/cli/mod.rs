//! Command-line front end. The only place that prints.

pub mod args;
pub mod commands;
pub mod render;

use std::fmt;
use std::io::{self, IsTerminal, Write};

use crate::converter::ConvertError;
use crate::event::Sink;
use crate::format::FormatError;
use crate::planner::PlanError;
use args::{ArgsError, Command, HELP, LogFormat, Verbosity};
use render::{HumanRenderer, JsonLinesRenderer};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitCode {
    Success = 0,
    Error = 1,
    Loss = 2,
    NoPath = 3,
    Usage = 4,
}

impl ExitCode {
    pub fn as_i32(self) -> i32 {
        self as i32
    }
}

#[derive(Debug)]
pub enum CliError {
    Args(ArgsError),
    /// A usage error the argument parser cannot see (a batch without --to).
    Usage(String),
    Format(FormatError),
    Plan(PlanError),
    Convert(ConvertError),
    Io {
        action: String,
        error: io::Error,
    },
}

impl CliError {
    pub fn exit_code(&self) -> ExitCode {
        match self {
            CliError::Args(_) => ExitCode::Usage,
            CliError::Usage(_) => ExitCode::Usage,
            CliError::Format(_) => ExitCode::Usage,
            CliError::Plan(PlanError::SameFormat(_)) => ExitCode::Usage,
            CliError::Plan(PlanError::NoPath { .. }) => ExitCode::NoPath,
            CliError::Convert(_) => ExitCode::Error,
            CliError::Io { .. } => ExitCode::Error,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CliError::Args(error) => write!(formatter, "{error}"),
            CliError::Usage(message) => write!(formatter, "{message}"),
            CliError::Format(error) => write!(formatter, "{error}"),
            CliError::Plan(error) => write!(formatter, "{error}"),
            CliError::Convert(error) => write!(formatter, "{error}"),
            CliError::Io { action, error } => write!(formatter, "{action}: {error}"),
        }
    }
}

impl From<ArgsError> for CliError {
    fn from(error: ArgsError) -> Self {
        CliError::Args(error)
    }
}

impl From<FormatError> for CliError {
    fn from(error: FormatError) -> Self {
        CliError::Format(error)
    }
}

impl From<PlanError> for CliError {
    fn from(error: PlanError) -> Self {
        CliError::Plan(error)
    }
}

impl From<ConvertError> for CliError {
    fn from(error: ConvertError) -> Self {
        CliError::Convert(error)
    }
}

/// Entry point used by `main`. Never panics; every failure maps to an exit code.
pub fn run(args: Vec<String>, env_log: Option<String>) -> ExitCode {
    let parsed = match args::parse(args) {
        Ok(parsed) => parsed,
        Err(error) => {
            let cli_error = CliError::Args(error);
            report_error(&cli_error, LogFormat::Human);
            return cli_error.exit_code();
        }
    };

    let verbosity = resolve_verbosity(parsed.global.verbosity, env_log.as_deref());
    let color = should_color(parsed.global.no_color);
    let log_format = parsed.global.log_format;
    let mut renderer: Box<dyn Sink> = match log_format {
        LogFormat::Human => Box::new(HumanRenderer::new(io::stderr(), verbosity, color)),
        LogFormat::Json => Box::new(JsonLinesRenderer::new(io::stderr())),
    };

    let stdout = io::stdout();
    let mut stdout_lock = stdout.lock();
    let result = match &parsed.command {
        Command::Help => {
            let written = stdout_lock.write_all(HELP.as_bytes());
            written
                .map(|_| ExitCode::Success)
                .map_err(|error| CliError::Io {
                    action: "writing help".to_string(),
                    error,
                })
        }
        Command::Version => {
            let version = env!("CARGO_PKG_VERSION");
            let written = writeln!(stdout_lock, "sublime {version}");
            written
                .map(|_| ExitCode::Success)
                .map_err(|error| CliError::Io {
                    action: "writing version".to_string(),
                    error,
                })
        }
        Command::Convert(convert_args) => {
            commands::convert::run(convert_args, renderer.as_mut(), &mut stdout_lock)
        }
        Command::Check(check_args) => {
            commands::check::run(check_args, log_format, &mut stdout_lock)
        }
        Command::Formats => commands::formats::run(log_format, &mut stdout_lock),
        Command::Paths { markdown } => {
            commands::paths::run(*markdown, log_format, &mut stdout_lock)
        }
        Command::Inspect(inspect_args) => commands::inspect::run(inspect_args, &mut stdout_lock),
    };
    let flushed = stdout_lock.flush();
    if let Err(error) = flushed {
        let cli_error = CliError::Io {
            action: "flushing stdout".to_string(),
            error,
        };
        report_error(&cli_error, log_format);
        return cli_error.exit_code();
    }

    match result {
        Ok(code) => code,
        Err(error) => {
            report_error(&error, log_format);
            error.exit_code()
        }
    }
}

fn resolve_verbosity(flag: Option<Verbosity>, env_log: Option<&str>) -> Verbosity {
    if let Some(verbosity) = flag {
        return verbosity;
    }
    if let Some(value) = env_log {
        if let Some(verbosity) = Verbosity::from_env_value(value) {
            return verbosity;
        }
    }
    Verbosity::Normal
}

fn should_color(no_color_flag: bool) -> bool {
    if no_color_flag {
        return false;
    }
    let env_disables = std::env::var_os("NO_COLOR").is_some();
    if env_disables {
        return false;
    }
    io::stderr().is_terminal()
}

fn report_error(error: &CliError, log_format: LogFormat) {
    let mut stderr = io::stderr();
    let message = error.to_string();
    let written = match log_format {
        LogFormat::Human => writeln!(stderr, "sublime: error: {message}"),
        LogFormat::Json => {
            let mut buffer: Vec<u8> = Vec::new();
            {
                let mut writer = crate::io::json::JsonWriter::new(&mut buffer);
                let _ = writer.begin_object();
                let _ = writer.key("event");
                let _ = writer.string("error");
                let _ = writer.key("message");
                let _ = writer.string(&message);
                let _ = writer.key("exit_code");
                let _ = writer.raw(&error.exit_code().as_i32().to_string());
                let _ = writer.end_object();
                let _ = writer.flush();
            }
            let line = String::from_utf8(buffer).unwrap_or_default();
            writeln!(stderr, "{line}")
        }
    };
    let _ = written;
}
