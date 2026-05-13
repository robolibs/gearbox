//! In-app log capture for the loader's tracing events. Ported
//! verbatim from bevy_openusd. Filters on usd_bevy / usd_schema /
//! usd_rapier / gearbox targets so the buffer doesn't fill with
//! framework noise.

use bevy::log::BoxedLayer;
use bevy::log::tracing::{self, Event, Level, Subscriber, field};
use bevy::log::tracing_subscriber::{Layer, layer::Context};
use bevy::prelude::*;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};

const MAX_LOG_LINES: usize = 500;

#[derive(Resource, Default, Clone)]
pub struct LoaderLog {
    pub buffer: Arc<Mutex<VecDeque<LogLine>>>,
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

impl<S> Layer<S> for LoaderLogLayer
where
    S: Subscriber,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let target = event.metadata().target();
        if !(target.starts_with("usd_bevy")
            || target.starts_with("usd_schema")
            || target.starts_with("usd_rapier")
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

pub fn loader_log_custom_layer(app: &mut App) -> Option<BoxedLayer> {
    let log = LoaderLog::default();
    let layer = LoaderLogLayer {
        buffer: Arc::clone(&log.buffer),
    };
    app.insert_resource(log);
    Some(Box::new(layer))
}

#[allow(dead_code)]
const _: () = {
    let _ = tracing::Level::TRACE;
};
