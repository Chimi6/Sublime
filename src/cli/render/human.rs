//! Human-readable event rendering with verbosity filtering and optional color.

use std::io::Write;

use crate::cli::args::Verbosity;
use crate::event::{Event, Hop, Sink};

const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const GREEN: &str = "\x1b[32m";
const DIM: &str = "\x1b[2m";
const RESET: &str = "\x1b[0m";

pub struct HumanRenderer<W: Write> {
    sink: W,
    verbosity: Verbosity,
    color: bool,
}

impl<W: Write> HumanRenderer<W> {
    pub fn new(sink: W, verbosity: Verbosity, color: bool) -> Self {
        HumanRenderer {
            sink,
            verbosity,
            color,
        }
    }

    pub fn into_inner(self) -> W {
        self.sink
    }

    fn line(&mut self, color: &str, label: &str, body: &str) {
        let written = if self.color {
            writeln!(self.sink, "{color}{label}{RESET} {body}")
        } else {
            writeln!(self.sink, "{label} {body}")
        };
        // Diagnostics failing to write (closed stderr) must never abort a conversion.
        let _ = written;
    }

    fn at_least(&self, level: Verbosity) -> bool {
        rank(self.verbosity) >= rank(level)
    }
}

fn rank(verbosity: Verbosity) -> u8 {
    match verbosity {
        Verbosity::Quiet => 0,
        Verbosity::Normal => 1,
        Verbosity::Verbose => 2,
        Verbosity::Debug => 3,
    }
}

fn describe_path(hops: &[Hop]) -> String {
    let from = hops.first().map(|hop| hop.from).unwrap_or("?");
    let to = hops.last().map(|hop| hop.to).unwrap_or("?");
    let names: Vec<&str> = hops.iter().map(|hop| hop.converter).collect();
    let mut worst = crate::converter::FidelityKind::Lossless;
    let mut descriptions: Vec<&str> = Vec::new();
    for hop in hops {
        let kind = hop.fidelity.kind();
        if kind > worst {
            worst = kind;
        }
        if let Some(text) = hop.fidelity.description() {
            descriptions.push(text);
        }
    }
    let fidelity = if descriptions.is_empty() {
        worst.label().to_string()
    } else {
        format!("{}: {}", worst.label(), descriptions.join("; "))
    };
    format!("{from} -> {to} via {} ({fidelity})", names.join(" -> "))
}

fn path_is_notable(hops: &[Hop]) -> bool {
    if hops.len() != 1 {
        return true;
    }
    !hops[0].fidelity.is_lossless()
}

impl<W: Write> Sink for HumanRenderer<W> {
    fn emit(&mut self, event: &Event) {
        match event {
            Event::Warning { message } => {
                if self.at_least(Verbosity::Normal) {
                    self.line(YELLOW, "warning:", message);
                }
            }
            Event::LossDetected {
                converter,
                location,
                description,
            } => {
                if self.at_least(Verbosity::Normal) {
                    let body = format!("{converter} at {location}: {description}");
                    self.line(RED, "loss:", &body);
                }
            }
            Event::PathChosen { hops } => {
                let show = self.at_least(Verbosity::Verbose)
                    || (self.at_least(Verbosity::Normal) && path_is_notable(hops));
                if show {
                    let body = describe_path(hops);
                    self.line(CYAN, "path:", &body);
                }
            }
            Event::StepStarted { converter } => {
                if self.at_least(Verbosity::Verbose) {
                    let body = format!("{converter} started");
                    self.line(GREEN, "step:", &body);
                }
            }
            Event::StepFinished { converter, elapsed } => {
                if self.at_least(Verbosity::Verbose) {
                    let milliseconds = elapsed.as_secs_f64() * 1000.0;
                    let body = format!("{converter} finished in {milliseconds:.3} ms");
                    self.line(GREEN, "step:", &body);
                }
            }
            Event::Progress {
                converter,
                bytes_read,
            } => {
                if self.at_least(Verbosity::Debug) {
                    let body = format!("{converter} read {bytes_read} bytes");
                    self.line(DIM, "progress:", &body);
                }
            }
            Event::Debug { message } => {
                if self.at_least(Verbosity::Debug) {
                    self.line(DIM, "debug:", message);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{Fidelity, Location, Tier};
    use crate::event::{Event, Hop, Sink};
    use std::time::Duration;

    fn render(verbosity: Verbosity, events: &[Event]) -> String {
        let mut renderer = HumanRenderer::new(Vec::new(), verbosity, false);
        for event in events {
            renderer.emit(event);
        }
        String::from_utf8(renderer.into_inner()).unwrap()
    }

    fn lossless_hop() -> Hop {
        Hop {
            converter: "csv-to-json",
            from: "csv",
            to: "json",
            fidelity: Fidelity::Lossless,
            tier: Tier::Native,
        }
    }

    fn lossy_hop() -> Hop {
        Hop {
            converter: "x-to-y",
            from: "x",
            to: "y",
            fidelity: Fidelity::Lossy("drops z"),
            tier: Tier::Native,
        }
    }

    #[test]
    fn quiet_prints_nothing() {
        let events = vec![Event::Warning {
            message: "w".to_string(),
        }];
        assert_eq!(render(Verbosity::Quiet, &events), "");
    }

    #[test]
    fn normal_prints_warnings_and_losses() {
        let events = vec![
            Event::Warning {
                message: "ragged".to_string(),
            },
            Event::LossDetected {
                converter: "csv-to-json",
                location: Location { line: 4, column: 3 },
                description: "dropped".to_string(),
            },
            Event::Debug {
                message: "hidden".to_string(),
            },
        ];
        let text = render(Verbosity::Normal, &events);
        assert_eq!(
            text,
            "warning: ragged\nloss: csv-to-json at line 4, column 3: dropped\n"
        );
    }

    #[test]
    fn normal_hides_direct_lossless_path_but_shows_lossy_path() {
        let direct = vec![Event::PathChosen {
            hops: vec![lossless_hop()],
        }];
        assert_eq!(render(Verbosity::Normal, &direct), "");
        let lossy = vec![Event::PathChosen {
            hops: vec![lossy_hop()],
        }];
        assert_eq!(
            render(Verbosity::Normal, &lossy),
            "path: x -> y via x-to-y (lossy: drops z)\n"
        );
    }

    #[test]
    fn verbose_prints_steps_and_every_path() {
        let events = vec![
            Event::PathChosen {
                hops: vec![lossless_hop()],
            },
            Event::StepStarted {
                converter: "csv-to-json",
            },
            Event::StepFinished {
                converter: "csv-to-json",
                elapsed: Duration::from_micros(1500),
            },
        ];
        let text = render(Verbosity::Verbose, &events);
        assert_eq!(
            text,
            "path: csv -> json via csv-to-json (lossless)\nstep: csv-to-json started\nstep: csv-to-json finished in 1.500 ms\n"
        );
    }

    #[test]
    fn debug_prints_everything() {
        let events = vec![
            Event::Debug {
                message: "d".to_string(),
            },
            Event::Progress {
                converter: "csv-to-json",
                bytes_read: 1024,
            },
        ];
        let text = render(Verbosity::Debug, &events);
        assert_eq!(text, "debug: d\nprogress: csv-to-json read 1024 bytes\n");
    }

    #[test]
    fn color_wraps_the_label() {
        let mut renderer = HumanRenderer::new(Vec::new(), Verbosity::Normal, true);
        renderer.emit(&Event::Warning {
            message: "w".to_string(),
        });
        let text = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(text, "\x1b[33mwarning:\x1b[0m w\n");
    }
}
