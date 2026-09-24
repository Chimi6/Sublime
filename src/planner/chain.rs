//! Runs a multi-hop plan as a chain of threads joined by pipes, so every
//! hop streams into the next without holding an intermediate whole.

use std::io::{self, BufWriter, Write};

use crate::converter::{ConvertError, ConvertOptions, Converter, Input};
use crate::event::{CollectingSink, Context};
use crate::planner::Plan;
use crate::planner::execute::run_hop;
use crate::planner::pipe::{PipeReader, PipeWriter, pipe};

const PIPE_CHUNK_SIZE: usize = 64 * 1024;

enum HopSource<'a> {
    Original(Input<'a>),
    Pipe(PipeReader),
}

struct HopOutcome {
    result: Result<(), ConvertError>,
    sink: CollectingSink,
}

pub(super) fn run_chain(
    plan: &Plan,
    input: Input<'_>,
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let hop_count = plan.hops.len();
    let options = context.options.clone();

    let outcomes: Vec<HopOutcome> = std::thread::scope(|scope| {
        let mut handles = Vec::with_capacity(hop_count - 1);
        let mut next_source = HopSource::Original(input);
        for hop_index in 0..hop_count - 1 {
            let converter = plan.hops[hop_index];
            let (pipe_writer, pipe_reader) = pipe();
            let source = std::mem::replace(&mut next_source, HopSource::Pipe(pipe_reader));
            let thread_options = options.clone();
            let handle = scope
                .spawn(move || run_hop_into_pipe(converter, source, pipe_writer, &thread_options));
            handles.push(handle);
        }
        let last_converter = plan.hops[hop_count - 1];
        let final_outcome = run_hop_collecting(last_converter, next_source, output, &options);

        let mut collected = Vec::with_capacity(hop_count);
        for handle in handles {
            let outcome = match handle.join() {
                Ok(outcome) => outcome,
                Err(_) => HopOutcome {
                    result: Err(ConvertError::Unsupported(
                        "a converter thread panicked".to_string(),
                    )),
                    sink: CollectingSink::new(),
                },
            };
            collected.push(outcome);
        }
        collected.push(final_outcome);
        collected
    });

    for outcome in &outcomes {
        for event in outcome.sink.events() {
            context.sink.emit(event);
        }
    }
    select_error(outcomes)
}

fn run_hop_into_pipe(
    converter: &'static dyn Converter,
    source: HopSource<'_>,
    pipe_writer: PipeWriter,
    options: &ConvertOptions,
) -> HopOutcome {
    let mut buffered = BufWriter::with_capacity(PIPE_CHUNK_SIZE, pipe_writer);
    let mut outcome = run_hop_collecting(converter, source, &mut buffered, options);
    let flush_result = buffered.flush();
    drop(buffered);
    if outcome.result.is_ok() {
        if let Err(error) = flush_result {
            outcome.result = Err(ConvertError::Io(error));
        }
    }
    outcome
}

fn run_hop_collecting(
    converter: &'static dyn Converter,
    source: HopSource<'_>,
    output: &mut dyn Write,
    options: &ConvertOptions,
) -> HopOutcome {
    let mut sink = CollectingSink::new();
    let result = {
        let mut context = Context::new(&mut sink, options);
        match source {
            HopSource::Original(input) => run_hop(converter, input, output, &mut context),
            HopSource::Pipe(mut reader) => {
                let input = Input::Stream(&mut reader);
                run_hop(converter, input, output, &mut context)
            }
        }
    };
    HopOutcome { result, sink }
}

fn select_error(outcomes: Vec<HopOutcome>) -> Result<(), ConvertError> {
    let mut first_error: Option<ConvertError> = None;
    for outcome in outcomes {
        let error = match outcome.result {
            Ok(()) => continue,
            Err(error) => error,
        };
        let is_broken_pipe = matches!(&error, ConvertError::Io(io_error) if io_error.kind() == io::ErrorKind::BrokenPipe);
        if !is_broken_pipe {
            return Err(error);
        }
        if first_error.is_none() {
            first_error = Some(error);
        }
    }
    match first_error {
        Some(error) => Err(error),
        None => Ok(()),
    }
}
