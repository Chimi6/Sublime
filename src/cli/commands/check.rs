//! `sublime check`: show the path and fidelity without converting.

use std::io::Write;

use crate::cli::args::{CheckArgs, LogFormat};
use crate::cli::render::json_lines::write_hop;
use crate::cli::{CliError, ExitCode};
use crate::format;
use crate::io::json::JsonWriter;
use crate::planner::{self, Plan, PlanOptions};
use crate::registry;

pub fn run(
    args: &CheckArgs,
    log_format: LogFormat,
    stdout: &mut dyn Write,
) -> Result<ExitCode, CliError> {
    let known = registry::all_formats();
    let from = format::find_by_id(&args.from, &known)?;
    let to = format::find_by_id(&args.to, &known)?;
    let options = PlanOptions {
        strict: args.strict,
        via: None,
    };
    let plan = planner::plan(registry::all_converters(), from, to, &options)?;

    let text = match log_format {
        LogFormat::Human => render_human(&plan),
        LogFormat::Json => render_json(&plan),
    };
    stdout
        .write_all(text.as_bytes())
        .map_err(|error| CliError::Io {
            action: "writing stdout".to_string(),
            error,
        })?;

    if plan.is_lossless() {
        Ok(ExitCode::Success)
    } else {
        Ok(ExitCode::Loss)
    }
}

pub fn render_human(plan: &Plan) -> String {
    let mut text = format!("{} -> {}\n", plan.from().id, plan.to().id);
    for (index, hop) in plan.describe().iter().enumerate() {
        let step = index + 1;
        let line = format!(
            "  {step}. {} ({}, {})\n",
            hop.converter,
            hop.tier.label(),
            hop.fidelity
        );
        text.push_str(&line);
    }
    let fidelity_line = format!("fidelity: {}\n", plan.worst_fidelity().label());
    text.push_str(&fidelity_line);
    text
}

pub fn render_json(plan: &Plan) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = JsonWriter::new(&mut buffer);
        let _ = writer.begin_object();
        let _ = writer.key("from");
        let _ = writer.string(plan.from().id);
        let _ = writer.key("to");
        let _ = writer.string(plan.to().id);
        let _ = writer.key("fidelity");
        let _ = writer.string(plan.worst_fidelity().label());
        let _ = writer.key("hops");
        let _ = writer.begin_array();
        for hop in plan.describe() {
            let _ = write_hop(&mut writer, &hop);
        }
        let _ = writer.end_array();
        let _ = writer.end_object();
        let _ = writer.flush();
    }
    let mut text = String::from_utf8(buffer).unwrap_or_default();
    text.push('\n');
    text
}
