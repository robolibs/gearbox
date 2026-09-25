//! In-app log capture for the Log pane. The mara host owns the tracing
//! subscriber (the embedded Bevy app runs without a `LogPlugin`), so the
//! layer is built there and its buffer is shared with the pane. Filters on
//! usd_bevy / usd_schema / gearbox targets so the buffer does
//! not fill with framework noise.

use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

use tracing::{Event, Level, Subscriber, field};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const MAX_LOG_LINES: usize = 500;

#[derive(Default, Clone)]
pub struct LoaderLog {
    pub buffer: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LoaderLog {
    /// The last `count` lines, oldest first, as `LEVEL target · message`.
    pub fn tail(&self, count: usize) -> Vec<String> {
        self.buffer
            .lock()
            .map(|buffer| {
                buffer
                    .iter()
                    .rev()
                    .take(count)
                    .rev()
                    .map(|line| {
                        let message: String = line.message.split_whitespace().collect::<Vec<_>>().join(" ");
                        let mut text = format!("{} {} · {message}", line.level, line.target);
                        if text.chars().count() > 160 {
                            text = text.chars().take(159).chain(std::iter::once('…')).collect();
                        }
                        text
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: Level,
    pub target: String,
    pub message: String,
}

struct LogVisitor(String);

impl field::Visit for LogVisitor {
    fn record_str(&mut self, fld: &field::Field, value: &str) {
        if fld.name() == "message" {
            self.0 = value.to_string();
        }
    }
    fn record_debug(&mut self, fld: &field::Field, value: &dyn fmt::Debug) {
        if fld.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

pub struct LoaderLogLayer {
    pub buffer: Arc<Mutex<VecDeque<LogLine>>>,
}

impl LoaderLogLayer {
    pub fn new(log: &LoaderLog) -> Self {
        Self {
            buffer: Arc::clone(&log.buffer),
        }
    }
}

impl<S> Layer<S> for LoaderLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        if !(target.starts_with("usd_bevy")
            || target.starts_with("usd_schema")
            || target.starts_with("gearbox"))
        {
            return;
        }
        let level = *event.metadata().level();
        if level > Level::INFO {
            return;
        }
        let mut visitor = LogVisitor(String::new());
        event.record(&mut visitor);
        let line = LogLine {
            level,
            target: target.to_string(),
            message: visitor.0,
        };
        if let Ok(mut buf) = self.buffer.lock() {
            buf.push_back(line);
            while buf.len() > MAX_LOG_LINES {
                buf.pop_front();
            }
        }
    }
}
