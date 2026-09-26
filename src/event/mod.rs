//! Typed events, sinks, and the context passed to converters.

use std::time::Duration;

use crate::converter::{ConvertOptions, Fidelity, Location, Tier};

/// One step of a chosen path, as reported to sinks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hop {
    pub converter: &'static str,
    pub from: &'static str,
    pub to: &'static str,
    pub fidelity: Fidelity,
    pub tier: Tier,
}

/// Everything the library can tell the outside world.
#[derive(Debug, Clone)]
pub enum Event {
    PathChosen {
        hops: Vec<Hop>,
    },
    StepStarted {
        converter: &'static str,
    },
    StepFinished {
        converter: &'static str,
        elapsed: Duration,
    },
    LossDetected {
        converter: &'static str,
        location: Location,
        description: String,
    },
    Progress {
        converter: &'static str,
        bytes_read: u64,
    },
    Warning {
        message: String,
    },
    Debug {
        message: String,
    },
    /// A batch has moved on to one file.
    FileStarted {
        input: String,
        output: String,
    },
    /// One file of a batch failed; the batch goes on.
    FileFailed {
        input: String,
        message: String,
    },
    /// The batch is over; on a dry run nothing was written and
    /// `converted` counts what would have been.
    BatchFinished {
        converted: u64,
        lossy: u64,
        failed: u64,
        skipped: u64,
        dry_run: bool,
    },
}

pub trait Sink {
    fn emit(&mut self, event: &Event);
}

/// Discards everything.
pub struct NullSink;

impl Sink for NullSink {
    fn emit(&mut self, _event: &Event) {}
}

/// Records every event. Produces the conversion report.
#[derive(Debug, Default)]
pub struct CollectingSink {
    events: Vec<Event>,
}

impl CollectingSink {
    pub fn new() -> Self {
        CollectingSink::default()
    }

    pub fn events(&self) -> &[Event] {
        &self.events
    }

    pub fn into_events(self) -> Vec<Event> {
        self.events
    }

    pub fn report(&self) -> ConversionReport {
        let mut losses = Vec::new();
        for event in &self.events {
            if let Event::LossDetected {
                converter,
                location,
                description,
            } = event
            {
                let record = LossRecord {
                    converter,
                    location: *location,
                    description: description.clone(),
                };
                losses.push(record);
            }
        }
        ConversionReport { losses }
    }
}

impl Sink for CollectingSink {
    fn emit(&mut self, event: &Event) {
        self.events.push(event.clone());
    }
}

/// Sends every event to several sinks in order.
pub struct MultiSink<'a> {
    sinks: Vec<&'a mut dyn Sink>,
}

impl<'a> MultiSink<'a> {
    pub fn new(sinks: Vec<&'a mut dyn Sink>) -> Self {
        MultiSink { sinks }
    }
}

impl Sink for MultiSink<'_> {
    fn emit(&mut self, event: &Event) {
        for sink in self.sinks.iter_mut() {
            sink.emit(event);
        }
    }
}

/// What a converter receives besides its input and output.
pub struct Context<'a> {
    pub sink: &'a mut dyn Sink,
    pub options: &'a ConvertOptions,
}

impl<'a> Context<'a> {
    pub fn new(sink: &'a mut dyn Sink, options: &'a ConvertOptions) -> Self {
        Context { sink, options }
    }

    pub fn emit(&mut self, event: Event) {
        self.sink.emit(&event);
    }

    pub fn loss(
        &mut self,
        converter: &'static str,
        location: Location,
        description: impl Into<String>,
    ) {
        let event = Event::LossDetected {
            converter,
            location,
            description: description.into(),
        };
        self.emit(event);
    }

    pub fn warning(&mut self, message: impl Into<String>) {
        let event = Event::Warning {
            message: message.into(),
        };
        self.emit(event);
    }

    pub fn debug(&mut self, message: impl Into<String>) {
        let event = Event::Debug {
            message: message.into(),
        };
        self.emit(event);
    }

    pub fn progress(&mut self, converter: &'static str, bytes_read: u64) {
        let event = Event::Progress {
            converter,
            bytes_read,
        };
        self.emit(event);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LossRecord {
    pub converter: &'static str,
    pub location: Location,
    pub description: String,
}

/// What actually happened during a conversion.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionReport {
    pub losses: Vec<LossRecord>,
}

impl ConversionReport {
    pub fn is_lossless(&self) -> bool {
        self.losses.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{ConvertOptions, Location};

    #[test]
    fn collecting_sink_builds_a_report_from_loss_events() {
        let options = ConvertOptions::default();
        let mut sink = CollectingSink::new();
        let mut context = Context::new(&mut sink, &options);
        context.warning("just a warning");
        context.loss("fake", Location { line: 2, column: 1 }, "dropped a field");
        let report = sink.report();
        assert_eq!(report.losses.len(), 1);
        assert_eq!(report.losses[0].converter, "fake");
        assert_eq!(report.losses[0].description, "dropped a field");
        assert!(!report.is_lossless());
        assert_eq!(sink.events().len(), 2);
    }

    #[test]
    fn multi_sink_fans_out() {
        let options = ConvertOptions::default();
        let mut first = CollectingSink::new();
        let mut second = CollectingSink::new();
        {
            let mut multi = MultiSink::new(vec![&mut first, &mut second]);
            let mut context = Context::new(&mut multi, &options);
            context.debug("hi");
        }
        assert_eq!(first.events().len(), 1);
        assert_eq!(second.events().len(), 1);
    }

    #[test]
    fn empty_report_is_lossless() {
        let sink = CollectingSink::new();
        assert!(sink.report().is_lossless());
    }
}
