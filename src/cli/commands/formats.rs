//! `sublime formats`: list every known format.

use std::io::Write;

use crate::cli::args::LogFormat;
use crate::cli::{CliError, ExitCode};
use crate::io::json::JsonWriter;
use crate::registry;

pub fn run(log_format: LogFormat, stdout: &mut dyn Write) -> Result<ExitCode, CliError> {
    let text = match log_format {
        LogFormat::Human => render_human(),
        LogFormat::Json => render_json(),
    };
    stdout
        .write_all(text.as_bytes())
        .map_err(|error| CliError::Io {
            action: "writing stdout".to_string(),
            error,
        })?;
    Ok(ExitCode::Success)
}

fn render_human() -> String {
    let mut text = String::new();
    for format in registry::all_formats() {
        let extensions = format.extensions.join(", ");
        let mut line = format!(
            "{:<14} {:<10} {:<32} extensions: {extensions}",
            format.id,
            format.category.label(),
            format.display_name
        );
        let labels = |options: &[crate::format::Setting]| -> String {
            options
                .iter()
                .map(|option| option.label())
                .collect::<Vec<_>>()
                .join(", ")
        };
        if !format.read_options.is_empty() {
            line.push_str(&format!("; read: {}", labels(format.read_options)));
        }
        if !format.write_options.is_empty() {
            line.push_str(&format!("; written: {}", labels(format.write_options)));
        }
        line.push('\n');
        text.push_str(&line);
    }
    text
}

fn render_json() -> String {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = JsonWriter::new(&mut buffer);
        let _ = writer.begin_array();
        for format in registry::all_formats() {
            let _ = writer.begin_object();
            let _ = writer.key("id");
            let _ = writer.string(format.id);
            let _ = writer.key("name");
            let _ = writer.string(format.display_name);
            let _ = writer.key("category");
            let _ = writer.string(format.category.label());
            let _ = writer.key("extensions");
            let _ = writer.begin_array();
            for extension in format.extensions {
                let _ = writer.string(extension);
            }
            let _ = writer.end_array();
            for (key, options) in [
                ("read_options", format.read_options),
                ("write_options", format.write_options),
            ] {
                let _ = writer.key(key);
                let _ = writer.begin_array();
                for option in options {
                    let _ = writer.string(option.flag());
                }
                let _ = writer.end_array();
            }
            let _ = writer.end_object();
        }
        let _ = writer.end_array();
        let _ = writer.flush();
    }
    let mut text = String::from_utf8(buffer).unwrap_or_default();
    text.push('\n');
    text
}
