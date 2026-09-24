//! `sublime inspect`: dump an iWork package's object graph. The work is in
//! `crate::inspect`, compiled only with the `dev-tools` feature; without it
//! the command explains how to get it.

use std::io::Write;

use crate::cli::args::InspectArgs;
use crate::cli::{CliError, ExitCode};

#[cfg(feature = "dev-tools")]
pub fn run(args: &InspectArgs, stdout: &mut dyn Write) -> Result<ExitCode, CliError> {
    let package = std::fs::read(&args.path).map_err(|error| CliError::Io {
        action: format!("reading {}", args.path),
        error,
    })?;
    let filter = crate::inspect::Filter {
        stream: args.stream.clone(),
        object: args.object,
        message_type: args.message_type,
        depth: args.depth,
    };
    let mut text = String::new();
    crate::inspect::dump(&package, &filter, &mut text).map_err(|message| CliError::Io {
        action: format!("inspecting {}", args.path),
        error: std::io::Error::other(message),
    })?;
    stdout
        .write_all(text.as_bytes())
        .map_err(|error| CliError::Io {
            action: "writing stdout".to_string(),
            error,
        })?;
    Ok(ExitCode::Success)
}

#[cfg(not(feature = "dev-tools"))]
pub fn run(_args: &InspectArgs, _stdout: &mut dyn Write) -> Result<ExitCode, CliError> {
    Err(CliError::Io {
        action: "inspect".to_string(),
        error: std::io::Error::other(
            "this build has no dev tools; build with --features dev-tools",
        ),
    })
}
