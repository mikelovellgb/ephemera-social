//! In-memory ring buffer log collector for the debug console.
//!
//! Implements a [`tracing_subscriber::Layer`] that captures the last N log
//! entries into a lock-protected [`VecDeque`]. The RPC endpoint
//! `meta.debug_log` reads from this buffer to surface logs in the frontend
//! debug panel — critical for pre-release debugging on both desktop and phone.

use serde::Serialize;
use std::collections::VecDeque;
use std::fmt;
use std::sync::{Arc, Mutex};
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

/// Maximum number of log entries kept in the ring buffer.
const DEFAULT_CAPACITY: usize = 200;

/// A single captured log entry.
#[derive(Debug, Clone, Serialize)]
pub struct LogEntry {
    /// Log level: "TRACE", "DEBUG", "INFO", "WARN", "ERROR".
    pub level: String,
    /// The module/target that produced the log line.
    pub target: String,
    /// The formatted log message.
    pub message: String,
    /// Wall-clock timestamp formatted as "HH:MM:SS".
    pub timestamp: String,
}

/// Thread-safe handle to the shared log buffer.
///
/// Clone this and pass it to both the tracing layer and the RPC handler.
#[derive(Clone)]
pub struct DebugLogHandle {
    inner: Arc<Mutex<VecDeque<LogEntry>>>,
}

impl DebugLogHandle {
    /// Create a new handle with the default capacity.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(VecDeque::with_capacity(DEFAULT_CAPACITY))),
        }
    }

    /// Read the last `n` entries (most recent last).
    ///
    /// Returns at most `n` entries. If `n` is 0 or exceeds the buffer size,
    /// all buffered entries are returned.
    pub fn read_last(&self, n: usize) -> Vec<LogEntry> {
        let guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if n == 0 || n >= guard.len() {
            guard.iter().cloned().collect()
        } else {
            guard.iter().skip(guard.len() - n).cloned().collect()
        }
    }

    /// Push a new entry, evicting the oldest if at capacity.
    fn push(&self, entry: LogEntry) {
        let mut guard = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if guard.len() >= DEFAULT_CAPACITY {
            guard.pop_front();
        }
        guard.push_back(entry);
    }
}

impl Default for DebugLogHandle {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for DebugLogHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DebugLogHandle").finish()
    }
}

/// A tracing [`Layer`] that captures log events into a [`DebugLogHandle`].
///
/// Add this alongside the normal `fmt` layer so logs go to both the
/// console/logcat AND the in-app debug panel.
pub struct DebugLogLayer {
    handle: DebugLogHandle,
}

impl DebugLogLayer {
    /// Create a new layer writing to the given handle.
    pub fn new(handle: DebugLogHandle) -> Self {
        Self { handle }
    }
}

/// Visitor that extracts the `message` field from a tracing event.
struct MessageVisitor {
    message: String,
}

impl MessageVisitor {
    fn new() -> Self {
        Self {
            message: String::new(),
        }
    }
}

impl Visit for MessageVisitor {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.message = format!("{:?}", value);
        } else if self.message.is_empty() {
            // Fallback: use the first field as the message.
            if !self.message.is_empty() {
                self.message.push(' ');
            }
            self.message = format!("{}={:?}", field.name(), value);
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.message = value.to_string();
        } else if self.message.is_empty() {
            self.message = format!("{}={}", field.name(), value);
        }
    }
}

impl<S: Subscriber> Layer<S> for DebugLogLayer {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let metadata = event.metadata();
        let level = *metadata.level();

        // Only capture INFO and above to avoid flooding the buffer with
        // debug/trace noise. Adjust if needed during development.
        if level > Level::DEBUG {
            return;
        }

        let level_str = match level {
            Level::ERROR => "ERROR",
            Level::WARN => "WARN",
            Level::INFO => "INFO",
            Level::DEBUG => "DEBUG",
            Level::TRACE => "TRACE",
        };

        let mut visitor = MessageVisitor::new();
        event.record(&mut visitor);

        // Format timestamp as HH:MM:SS using system local time.
        let now = std::time::SystemTime::now();
        let since_epoch = now
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let total_secs = since_epoch.as_secs();
        // Simple UTC-based HH:MM:SS (avoids pulling in chrono).
        let hours = (total_secs % 86400) / 3600;
        let minutes = (total_secs % 3600) / 60;
        let seconds = total_secs % 60;
        let timestamp = format!("{:02}:{:02}:{:02}", hours, minutes, seconds);

        self.handle.push(LogEntry {
            level: level_str.to_string(),
            target: metadata.target().to_string(),
            message: visitor.message,
            timestamp,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ring_buffer_evicts_oldest() {
        let handle = DebugLogHandle::new();
        for i in 0..250 {
            handle.push(LogEntry {
                level: "INFO".into(),
                target: "test".into(),
                message: format!("msg {}", i),
                timestamp: "00:00:00".into(),
            });
        }
        let entries = handle.read_last(0);
        assert_eq!(entries.len(), DEFAULT_CAPACITY);
        // Oldest should be msg 50 (0..49 were evicted)
        assert_eq!(entries[0].message, "msg 50");
        assert_eq!(entries[DEFAULT_CAPACITY - 1].message, "msg 249");
    }

    #[test]
    fn read_last_n() {
        let handle = DebugLogHandle::new();
        for i in 0..10 {
            handle.push(LogEntry {
                level: "INFO".into(),
                target: "test".into(),
                message: format!("msg {}", i),
                timestamp: "00:00:00".into(),
            });
        }
        let last_3 = handle.read_last(3);
        assert_eq!(last_3.len(), 3);
        assert_eq!(last_3[0].message, "msg 7");
        assert_eq!(last_3[2].message, "msg 9");
    }
}
