//! One JSON object per event on stderr. The contract for other programs.

use std::io::Write;

use crate::event::{Event, Hop, Sink};
use crate::io::json::JsonWriter;

pub struct JsonLinesRenderer<W: Write> {
    sink: W,
}

impl<W: Write> JsonLinesRenderer<W> {
    pub fn new(sink: W) -> Self {
        JsonLinesRenderer { sink }
    }

    pub fn into_inner(self) -> W {
        self.sink
    }
}

impl<W: Write> Sink for JsonLinesRenderer<W> {
    fn emit(&mut self, event: &Event) {
        let line = encode_event(event);
        let written = writeln!(self.sink, "{line}");
        let _ = written;
    }
}

/// Encodes one event as a compact JSON object. Field names are a stable
/// contract for other programs; change them only with a major version bump.
pub fn encode_event(event: &Event) -> String {
    let mut buffer: Vec<u8> = Vec::new();
    {
        let mut writer = JsonWriter::new(&mut buffer);
        // Writing to a Vec cannot fail; the results are ignored on purpose.
        let _ = write_event(&mut writer, event);
        let _ = writer.flush();
    }
    String::from_utf8(buffer).unwrap_or_default()
}

fn write_event(writer: &mut JsonWriter<&mut Vec<u8>>, event: &Event) -> std::io::Result<()> {
    writer.begin_object()?;
    writer.key("event")?;
    match event {
        Event::PathChosen { hops } => {
            writer.string("path_chosen")?;
            writer.key("hops")?;
            writer.begin_array()?;
            for hop in hops {
                write_hop(writer, hop)?;
            }
            writer.end_array()?;
        }
        Event::StepStarted { converter } => {
            writer.string("step_started")?;
            writer.key("converter")?;
            writer.string(converter)?;
        }
        Event::StepFinished { converter, elapsed } => {
            writer.string("step_finished")?;
            writer.key("converter")?;
            writer.string(converter)?;
            writer.key("elapsed_ms")?;
            let milliseconds = elapsed.as_millis().to_string();
            writer.raw(&milliseconds)?;
        }
        Event::LossDetected {
            converter,
            location,
            description,
        } => {
            writer.string("loss")?;
            writer.key("converter")?;
            writer.string(converter)?;
            writer.key("line")?;
            writer.raw(&location.line.to_string())?;
            writer.key("column")?;
            writer.raw(&location.column.to_string())?;
            writer.key("description")?;
            writer.string(description)?;
        }
        Event::Progress {
            converter,
            bytes_read,
        } => {
            writer.string("progress")?;
            writer.key("converter")?;
            writer.string(converter)?;
            writer.key("bytes_read")?;
            writer.raw(&bytes_read.to_string())?;
        }
        Event::Warning { message } => {
            writer.string("warning")?;
            writer.key("message")?;
            writer.string(message)?;
        }
        Event::Debug { message } => {
            writer.string("debug")?;
            writer.key("message")?;
            writer.string(message)?;
        }
    }
    writer.end_object()
}

pub fn write_hop(writer: &mut JsonWriter<&mut Vec<u8>>, hop: &Hop) -> std::io::Result<()> {
    writer.begin_object()?;
    writer.key("converter")?;
    writer.string(hop.converter)?;
    writer.key("from")?;
    writer.string(hop.from)?;
    writer.key("to")?;
    writer.string(hop.to)?;
    writer.key("tier")?;
    writer.string(hop.tier.label())?;
    writer.key("fidelity")?;
    writer.string(hop.fidelity.kind().label())?;
    writer.key("description")?;
    match hop.fidelity.description() {
        Some(text) => writer.string(text)?,
        None => writer.null()?,
    }
    writer.end_object()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::converter::{Fidelity, Location, Tier};
    use crate::event::{Event, Hop, Sink};
    use std::time::Duration;

    #[test]
    fn encodes_each_event_kind() {
        let path = Event::PathChosen {
            hops: vec![Hop {
                converter: "a-b",
                from: "a",
                to: "b",
                fidelity: Fidelity::Conditional("when"),
                tier: Tier::Native,
            }],
        };
        assert_eq!(
            encode_event(&path),
            "{\"event\":\"path_chosen\",\"hops\":[{\"converter\":\"a-b\",\"from\":\"a\",\"to\":\"b\",\"tier\":\"native\",\"fidelity\":\"conditional\",\"description\":\"when\"}]}"
        );
        let started = Event::StepStarted { converter: "a-b" };
        assert_eq!(
            encode_event(&started),
            "{\"event\":\"step_started\",\"converter\":\"a-b\"}"
        );
        let finished = Event::StepFinished {
            converter: "a-b",
            elapsed: Duration::from_millis(2),
        };
        assert_eq!(
            encode_event(&finished),
            "{\"event\":\"step_finished\",\"converter\":\"a-b\",\"elapsed_ms\":2}"
        );
        let loss = Event::LossDetected {
            converter: "a-b",
            location: Location { line: 1, column: 2 },
            description: "x \"y\"".to_string(),
        };
        assert_eq!(
            encode_event(&loss),
            "{\"event\":\"loss\",\"converter\":\"a-b\",\"line\":1,\"column\":2,\"description\":\"x \\\"y\\\"\"}"
        );
        let progress = Event::Progress {
            converter: "a-b",
            bytes_read: 7,
        };
        assert_eq!(
            encode_event(&progress),
            "{\"event\":\"progress\",\"converter\":\"a-b\",\"bytes_read\":7}"
        );
        let warning = Event::Warning {
            message: "m".to_string(),
        };
        assert_eq!(
            encode_event(&warning),
            "{\"event\":\"warning\",\"message\":\"m\"}"
        );
        let debug = Event::Debug {
            message: "m".to_string(),
        };
        assert_eq!(
            encode_event(&debug),
            "{\"event\":\"debug\",\"message\":\"m\"}"
        );
    }

    #[test]
    fn lossless_hop_has_null_description() {
        let path = Event::PathChosen {
            hops: vec![Hop {
                converter: "a-b",
                from: "a",
                to: "b",
                fidelity: Fidelity::Lossless,
                tier: Tier::External,
            }],
        };
        assert!(
            encode_event(&path)
                .contains("\"tier\":\"external\",\"fidelity\":\"lossless\",\"description\":null")
        );
    }

    #[test]
    fn renderer_writes_one_line_per_event() {
        let mut renderer = JsonLinesRenderer::new(Vec::new());
        renderer.emit(&Event::Warning {
            message: "a".to_string(),
        });
        renderer.emit(&Event::Warning {
            message: "b".to_string(),
        });
        let text = String::from_utf8(renderer.into_inner()).unwrap();
        assert_eq!(
            text,
            "{\"event\":\"warning\",\"message\":\"a\"}\n{\"event\":\"warning\",\"message\":\"b\"}\n"
        );
    }
}
