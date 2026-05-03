//! Unified logging: stderr + rotating file on disk + in-memory ring buffer for UI.
//!
//! The desktop app is the single sink for all logs. The sidecar's stderr is
//! already forwarded into this subscriber via
//! `yapstack_transcription::client::stderr_reader_task` (as tracing events with
//! target `yapstack_transcription_sidecar`), so we do not need a separate
//! appender in the sidecar process.
//!
//! PII contract: log call sites must never include transcript text. Filter
//! call sites in `crates/yapstack-transcription-sidecar/src/engines/*` already log only
//! lengths. If you add new call sites, follow the same rule.

use std::collections::VecDeque;
use std::fmt::Write as _;
use std::path::Path;
use std::sync::{Arc, Mutex};

use serde::{Deserialize, Serialize};
use specta::Type;
use tauri::{AppHandle, Emitter};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt;
use tracing_subscriber::layer::{Context, SubscriberExt};
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{EnvFilter, Layer};

/// Tauri event name used to stream log entries to the frontend.
pub const LOG_EVENT_NAME: &str = "log://entry";

/// Mirrors the five `tracing::Level` values so the frontend gets a
/// typed union instead of an unchecked string.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Type)]
#[serde(rename_all = "UPPERCASE")]
pub enum LogLevel {
    Error,
    Warn,
    Info,
    Debug,
    Trace,
}

impl From<&Level> for LogLevel {
    fn from(level: &Level) -> Self {
        match *level {
            Level::ERROR => LogLevel::Error,
            Level::WARN => LogLevel::Warn,
            Level::INFO => LogLevel::Info,
            Level::DEBUG => LogLevel::Debug,
            Level::TRACE => LogLevel::Trace,
        }
    }
}

/// One log line visible to the frontend. `ts_ms` is unix-epoch milliseconds.
#[derive(Debug, Clone, Serialize, Deserialize, Type)]
pub struct LogEntry {
    pub ts_ms: i64,
    pub level: LogLevel,
    pub target: String,
    pub message: String,
}

/// Bounded in-memory buffer of recent entries. Oldest entries drop when full.
pub struct LogBuffer {
    inner: Mutex<VecDeque<LogEntry>>,
    capacity: usize,
}

impl LogBuffer {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Mutex::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    fn push(&self, entry: LogEntry) {
        let mut guard = self.inner.lock().expect("log buffer mutex poisoned");
        if guard.len() == self.capacity {
            guard.pop_front();
        }
        guard.push_back(entry);
    }

    pub fn snapshot(&self, limit: usize) -> Vec<LogEntry> {
        let guard = self.inner.lock().expect("log buffer mutex poisoned");
        let len = guard.len();
        let take = limit.min(len);
        guard.iter().skip(len - take).cloned().collect()
    }

    pub fn clear(&self) {
        self.inner
            .lock()
            .expect("log buffer mutex poisoned")
            .clear();
    }
}

/// Tracing `Layer` that appends each event into a `LogBuffer` and emits a
/// Tauri event so the frontend can tail logs live.
pub struct RingBufferLayer {
    buffer: Arc<LogBuffer>,
    app: AppHandle,
}

impl RingBufferLayer {
    pub fn new(buffer: Arc<LogBuffer>, app: AppHandle) -> Self {
        Self { buffer, app }
    }
}

/// Field visitor that renders the event's `message` plus any `key=value` kv
/// fields into a single human-readable string. Mirrors `tracing-subscriber`'s
/// default `fmt` layout enough to be familiar without pulling in that machinery.
#[derive(Default)]
struct MessageVisitor {
    message: String,
    extras: String,
}

impl MessageVisitor {
    fn render(mut self) -> String {
        if self.extras.is_empty() {
            self.message
        } else if self.message.is_empty() {
            self.extras.trim_start().to_string()
        } else {
            let _ = write!(self.message, " {}", self.extras.trim_start());
            self.message
        }
    }

    fn append_kv(&mut self, name: &str, value: impl std::fmt::Display) {
        let _ = write!(self.extras, " {name}={value}");
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.message, "{value:?}");
        } else {
            self.append_kv(field.name(), format_args!("{value:?}"));
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message.push_str(value);
        } else {
            self.append_kv(field.name(), value);
        }
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.append_kv(field.name(), value);
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.append_kv(field.name(), value);
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.append_kv(field.name(), value);
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.append_kv(field.name(), value);
    }
}

impl<S> Layer<S> for RingBufferLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let entry = LogEntry {
            ts_ms: chrono::Utc::now().timestamp_millis(),
            level: meta.level().into(),
            target: meta.target().to_string(),
            message: visitor.render(),
        };
        self.buffer.push(entry.clone());
        // Best-effort: frontend may not be mounted yet, or event system down.
        let _ = self.app.emit(LOG_EVENT_NAME, &entry);
    }
}

/// Default per-crate filter. Can be overridden via the `RUST_LOG` env var.
const DEFAULT_FILTER: &str = "info,\
    yapstack_desktop=debug,\
    yapstack_audio=info,\
    yapstack_transcription=debug,\
    yapstack_transcription_sidecar=debug,\
    tao=warn,\
    wry=warn,\
    hyper=warn,\
    reqwest=warn,\
    sqlx=warn";

/// Install the global tracing subscriber with three layers:
///   1. stderr fmt (developer console)
///   2. rolling daily file under `log_dir` (`yapstack.log.YYYY-MM-DD`)
///   3. in-memory ring buffer + Tauri `log://entry` event stream
///
/// The returned `WorkerGuard` must be kept alive for the lifetime of the
/// process — drop it and the non-blocking writer thread flushes and exits.
pub fn init(log_dir: &Path, app: AppHandle) -> (Arc<LogBuffer>, WorkerGuard) {
    let file_appender = tracing_appender::rolling::daily(log_dir, "yapstack.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(DEFAULT_FILTER));

    let buffer = Arc::new(LogBuffer::new(500));

    let stderr_layer = fmt::layer().with_writer(std::io::stderr).with_target(true);
    let file_layer = fmt::layer()
        .with_writer(non_blocking)
        .with_ansi(false)
        .with_target(true);
    let ring_layer = RingBufferLayer::new(buffer.clone(), app);

    let _ = tracing_subscriber::registry()
        .with(filter)
        .with(stderr_layer)
        .with(file_layer)
        .with(ring_layer)
        .try_init();

    tracing::info!(log_dir = ?log_dir, "logging initialised");
    (buffer, guard)
}
