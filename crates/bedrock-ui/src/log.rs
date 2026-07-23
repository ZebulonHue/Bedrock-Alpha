//! In-memory log buffer feeding the Output Log panel, plus the tracing layer
//! that mirrors every event to a human-readable log file.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;

/// Severity of a log line, mirrored from `tracing::Level`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Level {
    /// Something went wrong.
    Error,
    /// Something looks off but work continues.
    Warn,
    /// Normal operational message.
    Info,
    /// Developer detail.
    Debug,
    /// Very verbose developer detail.
    Trace,
}

impl Level {
    /// Short prefix used in the log file.
    pub fn label(self) -> &'static str {
        match self {
            Level::Error => "ERROR",
            Level::Warn => "WARN",
            Level::Info => "INFO",
            Level::Debug => "DEBUG",
            Level::Trace => "TRACE",
        }
    }
}

/// Thread-safe ring buffer of log lines shown by the Output Log panel.
#[derive(Clone)]
pub struct LogBuffer {
    inner: Arc<Mutex<VecDeque<(Level, String)>>>,
    capacity: usize,
}

impl LogBuffer {
    /// Create a buffer keeping at most `capacity` lines.
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(capacity))),
            capacity,
        }
    }

    /// Append a line, dropping the oldest when full.
    pub fn push(&self, level: Level, line: impl Into<String>) {
        let mut inner = self.inner.lock().unwrap();
        if inner.len() >= self.capacity {
            inner.pop_front();
        }
        inner.push_back((level, line.into()));
    }

    /// Snapshot of all buffered lines, oldest first.
    pub fn lines(&self) -> Vec<(Level, String)> {
        self.inner.lock().unwrap().iter().cloned().collect()
    }

    /// Remove all buffered lines.
    pub fn clear(&self) {
        self.inner.lock().unwrap().clear();
    }
}

/// Extracts the `message` field from a tracing event.
#[derive(Default)]
struct MessageVisitor(String);

impl Visit for MessageVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0 = value.to_owned();
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

/// tracing layer forwarding events to a [`LogBuffer`] and a log file.
struct BufferLayer {
    buffer: LogBuffer,
    file: Mutex<Option<File>>,
}

impl<S: tracing::Subscriber> Layer<S> for BufferLayer {
    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);

        let metadata = event.metadata();
        let level = if *metadata.level() == tracing::Level::ERROR {
            Level::Error
        } else if *metadata.level() == tracing::Level::WARN {
            Level::Warn
        } else if *metadata.level() == tracing::Level::DEBUG {
            Level::Debug
        } else if *metadata.level() == tracing::Level::TRACE {
            Level::Trace
        } else {
            Level::Info
        };

        let line = format!("{}: {}", metadata.target(), visitor.0);
        self.buffer.push(level, line.clone());
        if let Some(file) = self.file.lock().unwrap().as_mut() {
            let _ = writeln!(file, "{} {line}", level.label());
        }
    }
}

/// Install the global tracing subscriber. Events at INFO and above are kept;
/// the log file is truncated on every launch. Call once, early in `main`.
pub fn init(buffer: LogBuffer, log_file: &Path) {
    if let Some(parent) = log_file.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let layer = BufferLayer {
        buffer,
        file: Mutex::new(File::create(log_file).ok()),
    };
    tracing_subscriber::registry()
        .with(layer.with_filter(tracing_subscriber::filter::LevelFilter::INFO))
        .init();
}
