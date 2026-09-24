//! Runs a plan: one hop directly, several hops as a threaded chain, or,
//! where there are no threads and no clock (WebAssembly), as a sequence
//! of in-memory hops.

use std::io::Write;
#[cfg(not(target_arch = "wasm32"))]
use std::time::Instant;

use crate::converter::{ConvertError, Converter, Input};
use crate::event::{Context, Event};
use crate::planner::Plan;
#[cfg(not(target_arch = "wasm32"))]
use crate::planner::chain::run_chain;

pub fn execute(
    plan: &Plan,
    input: Input<'_>,
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    if plan.hops.is_empty() {
        return Err(ConvertError::Unsupported(
            "plan has no conversion steps".to_string(),
        ));
    }
    if plan.hops.len() == 1 {
        return run_hop(plan.hops[0], input, output, context);
    }
    #[cfg(target_arch = "wasm32")]
    {
        execute_in_memory(plan, input, output, context)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        run_chain(plan, input, output, context)
    }
}

/// Runs a chain one hop at a time through in-memory buffers: no threads,
/// so it works everywhere, at the cost of holding each intermediate whole.
pub fn execute_in_memory(
    plan: &Plan,
    mut input: Input<'_>,
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    let Some((last, first_hops)) = plan.hops.split_last() else {
        return Err(ConvertError::Unsupported(
            "plan has no conversion steps".to_string(),
        ));
    };
    let mut buffer = Vec::new();
    let mut is_first = true;
    for converter in first_hops {
        let mut next = Vec::new();
        if is_first {
            run_hop(*converter, Input::Stream(&mut input), &mut next, context)?;
            is_first = false;
        } else {
            let mut source: &[u8] = &buffer;
            run_hop(*converter, Input::Stream(&mut source), &mut next, context)?;
        }
        buffer = next;
    }
    if is_first {
        run_hop(*last, input, output, context)
    } else {
        let mut source: &[u8] = &buffer;
        run_hop(*last, Input::Stream(&mut source), output, context)
    }
}

pub(super) fn run_hop(
    converter: &'static dyn Converter,
    input: Input<'_>,
    output: &mut dyn Write,
    context: &mut Context<'_>,
) -> Result<(), ConvertError> {
    context.emit(Event::StepStarted {
        converter: converter.name(),
    });
    #[cfg(not(target_arch = "wasm32"))]
    let started = Instant::now();
    let result = converter.convert(input, output, context);
    #[cfg(not(target_arch = "wasm32"))]
    let elapsed = started.elapsed();
    #[cfg(target_arch = "wasm32")]
    let elapsed = std::time::Duration::ZERO;
    context.emit(Event::StepFinished {
        converter: converter.name(),
        elapsed,
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{ConvertOptions, Converter, Fidelity, Input, Tier};
    use crate::event::{CollectingSink, Event};
    use crate::format::{Category, Format};
    use std::io::{Read, Write};

    static A: Format = Format {
        id: "a",
        display_name: "A",
        extensions: &["a"],
        magic: None,
        category: Category::Data,
    };
    static B: Format = Format {
        id: "b",
        display_name: "B",
        extensions: &["b"],
        magic: None,
        category: Category::Data,
    };
    static C: Format = Format {
        id: "c",
        display_name: "C",
        extensions: &["c"],
        magic: None,
        category: Category::Data,
    };
    static D: Format = Format {
        id: "d",
        display_name: "D",
        extensions: &["d"],
        magic: None,
        category: Category::Data,
    };

    struct Upper;
    struct Exclaim;
    struct Fails;
    struct Tail;
    struct WritesALot;

    impl Converter for Upper {
        fn name(&self) -> &'static str {
            "upper"
        }
        fn from(&self) -> &'static Format {
            &A
        }
        fn to(&self) -> &'static Format {
            &B
        }
        fn fidelity(&self) -> Fidelity {
            Fidelity::Lossless
        }
        fn tier(&self) -> Tier {
            Tier::Native
        }
        fn convert(
            &self,
            mut input: Input<'_>,
            output: &mut dyn Write,
            context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            let mut text = String::new();
            input.read_to_string(&mut text)?;
            context.debug("upper ran");
            output.write_all(text.to_uppercase().as_bytes())?;
            Ok(())
        }
    }

    impl Converter for Exclaim {
        fn name(&self) -> &'static str {
            "exclaim"
        }
        fn from(&self) -> &'static Format {
            &B
        }
        fn to(&self) -> &'static Format {
            &C
        }
        fn fidelity(&self) -> Fidelity {
            Fidelity::Lossless
        }
        fn tier(&self) -> Tier {
            Tier::Native
        }
        fn convert(
            &self,
            mut input: Input<'_>,
            output: &mut dyn Write,
            context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            let mut text = String::new();
            input.read_to_string(&mut text)?;
            context.debug("exclaim ran");
            output.write_all(text.as_bytes())?;
            output.write_all(b"!")?;
            Ok(())
        }
    }

    impl Converter for Fails {
        fn name(&self) -> &'static str {
            "fails"
        }
        fn from(&self) -> &'static Format {
            &B
        }
        fn to(&self) -> &'static Format {
            &C
        }
        fn fidelity(&self) -> Fidelity {
            Fidelity::Lossless
        }
        fn tier(&self) -> Tier {
            Tier::Native
        }
        fn convert(
            &self,
            _input: Input<'_>,
            _output: &mut dyn Write,
            _context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            Err(ConvertError::Unsupported("boom".to_string()))
        }
    }

    impl Converter for Tail {
        fn name(&self) -> &'static str {
            "tail"
        }
        fn from(&self) -> &'static Format {
            &C
        }
        fn to(&self) -> &'static Format {
            &D
        }
        fn fidelity(&self) -> Fidelity {
            Fidelity::Lossless
        }
        fn tier(&self) -> Tier {
            Tier::Native
        }
        fn convert(
            &self,
            mut input: Input<'_>,
            output: &mut dyn Write,
            context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            let mut buffer = Vec::new();
            input.read_to_end(&mut buffer)?;
            context.debug("tail ran");
            output.write_all(&buffer)?;
            output.write_all(b"?")?;
            Ok(())
        }
    }

    impl Converter for WritesALot {
        fn name(&self) -> &'static str {
            "writes_a_lot"
        }
        fn from(&self) -> &'static Format {
            &A
        }
        fn to(&self) -> &'static Format {
            &B
        }
        fn fidelity(&self) -> Fidelity {
            Fidelity::Lossless
        }
        fn tier(&self) -> Tier {
            Tier::Native
        }
        fn convert(
            &self,
            _input: Input<'_>,
            output: &mut dyn Write,
            _context: &mut Context<'_>,
        ) -> Result<(), ConvertError> {
            let payload = vec![b'x'; 1024 * 1024];
            output.write_all(&payload)?;
            Ok(())
        }
    }

    static UPPER: Upper = Upper;
    static EXCLAIM: Exclaim = Exclaim;
    static FAILS: Fails = Fails;
    static TAIL: Tail = Tail;
    static WRITES_A_LOT: WritesALot = WritesALot;

    fn run(plan: &Plan, input: &[u8]) -> (Result<(), ConvertError>, Vec<u8>, CollectingSink) {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut output = Vec::new();
        let result = {
            let mut context = Context::new(&mut sink, &options);
            let mut source: &[u8] = input;
            execute(plan, Input::Stream(&mut source), &mut output, &mut context)
        };
        (result, output, sink)
    }

    fn step_names(sink: &CollectingSink) -> Vec<String> {
        let mut names = Vec::new();
        for event in sink.events() {
            match event {
                Event::StepStarted { converter } => names.push(format!("start {converter}")),
                Event::StepFinished { converter, .. } => names.push(format!("finish {converter}")),
                Event::Debug { message } => names.push(message.clone()),
                _ => {}
            }
        }
        names
    }

    #[test]
    fn single_hop_runs_directly() {
        let plan = Plan { hops: vec![&UPPER] };
        let (result, output, sink) = run(&plan, b"hi");
        result.unwrap();
        assert_eq!(output, b"HI");
        assert_eq!(
            step_names(&sink),
            vec!["start upper", "upper ran", "finish upper"]
        );
    }

    #[test]
    fn two_hops_chain_through_a_pipe_and_replay_events_in_order() {
        let plan = Plan {
            hops: vec![&UPPER, &EXCLAIM],
        };
        let (result, output, sink) = run(&plan, b"hi");
        result.unwrap();
        assert_eq!(output, b"HI!");
        assert_eq!(
            step_names(&sink),
            vec![
                "start upper",
                "upper ran",
                "finish upper",
                "start exclaim",
                "exclaim ran",
                "finish exclaim"
            ]
        );
    }

    #[test]
    fn downstream_failure_is_reported_not_broken_pipe() {
        let plan = Plan {
            hops: vec![&UPPER, &FAILS],
        };
        let (result, _, _) = run(&plan, b"hi");
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "boom");
    }

    #[test]
    fn empty_plan_is_an_error() {
        let plan = Plan { hops: vec![] };
        let (result, _, _) = run(&plan, b"hi");
        let error = result.unwrap_err();
        assert!(matches!(error, ConvertError::Unsupported(_)));
    }

    fn step_lifecycle_events(sink: &CollectingSink) -> Vec<String> {
        let mut names = Vec::new();
        for event in sink.events() {
            match event {
                Event::StepStarted { converter } => names.push(format!("start {converter}")),
                Event::StepFinished { converter, .. } => names.push(format!("finish {converter}")),
                _ => {}
            }
        }
        names
    }

    #[test]
    fn three_hops_stream_a_large_payload() {
        let plan = Plan {
            hops: vec![&UPPER, &EXCLAIM, &TAIL],
        };
        let input = vec![b'a'; 1024 * 1024];
        let (result, output, sink) = run(&plan, &input);
        result.unwrap();
        let expected_prefix = vec![b'A'; 1024 * 1024];
        assert_eq!(&output[..1024 * 1024], expected_prefix.as_slice());
        assert_eq!(&output[1024 * 1024..], b"!?");
        assert_eq!(
            step_lifecycle_events(&sink),
            vec![
                "start upper",
                "finish upper",
                "start exclaim",
                "finish exclaim",
                "start tail",
                "finish tail",
            ]
        );
    }

    #[test]
    fn upstream_broken_pipe_is_suppressed_in_favor_of_downstream_error() {
        let plan = Plan {
            hops: vec![&WRITES_A_LOT, &FAILS],
        };
        let (result, _, _) = run(&plan, b"hi");
        let error = result.unwrap_err();
        assert_eq!(error.to_string(), "boom");
    }
}
