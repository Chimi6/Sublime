//! Comparison harness. Runs our converters and the conventional-crate
//! pipelines on the same inputs. Never part of the shipped binary.
//!
//! Usage:
//!   bench startup <binary> <csv>       median ms from spawn to first output byte
//!   bench spawn-baseline <binary>      median ms to spawn and exit (the floor)
//!   bench <pair> <mode> [args...]      a pair's generator, pipelines, or reader
//!
//! Pairs live in `src/pairs/`, one module per conversion pair.

mod common;
mod pairs;
mod startup;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.as_slice() {
        [command, binary, input] if command == "startup" => startup::measure_startup(binary, input),
        [command, binary] if command == "spawn-baseline" => startup::measure_spawn_baseline(binary),
        [pair, mode, rest @ ..] => pairs::run(pair, mode, rest),
        _ => {
            eprintln!(
                "usage: bench startup <binary> <csv> | spawn-baseline <binary> | <pair> <mode> [args...]"
            );
            eprintln!("pairs: {}", pairs::NAMES.join(", "));
            std::process::exit(4);
        }
    };
    if let Err(error) = result {
        eprintln!("bench: {error}");
        std::process::exit(1);
    }
}
