//! Optional async transport audit logger.
//!
//! Enabled with the `transport-log` Cargo feature (off by default).
//!
//! # Overview
//!
//! [`TransportLogger`] records a [`LogEntry`] for every task request sent or
//! received on either transport. Entries are written asynchronously via a
//! bounded [`tokio::sync::mpsc`] channel to a dedicated background Tokio task
//! that appends them as **newline-delimited JSON (NDJSON)** to rotating files.
//!
//! The transport hot path only calls [`mpsc::Sender::try_send`], which is
//! **non-blocking** and never parks the calling task. If the channel is full
//! the entry is silently dropped — the transport latency is never affected by
//! logging I/O.
//!
//! # Log file format
//!
//! Files are written to the configured directory with names like:
//!
//! ```text
//! transport-2026-06-08T09-51-00.123456Z.ndjson
//! transport-2026-06-08T09-52-01.456789Z.ndjson
//! …
//! ```
//!
//! Each line is a self-contained JSON object (fields omitted when `null`):
//!
//! ```text
//! {"ts":"2026-06-08T09:51:00.123456Z","transport":"fast","direction":"outbound","task_id":"…","duration_us":84,"status":"ok"}
//! {"ts":"2026-06-08T09:51:00.456Z","transport":"a2a","direction":"outbound","task_id":"…","duration_us":4231,"status":"error","error":"JSON-RPC error -32600"}
//! ```
//!
//! A new file is opened once the previous one reaches `max_entries` lines
//! (default: **1 000**).
//!
//! # Quick start
//!
//! ```rust,ignore
//! use messaggero::TransportLogger;
//!
//! let logger = TransportLogger::builder()
//!     .log_dir("/var/log/messaggero")
//!     .build()
//!     .await?;
//!
//! // Server-side: wraps the agent and records every inbound task
//! messaggero::serve(MyAgent)
//!     .fast("/tmp/agent.sock")
//!     .http("127.0.0.1:3000")
//!     .with_transport_logger(logger.clone())
//!     .run()
//!     .await?;
//!
//! // Router (client-side): records every outbound task dispatch
//! let router = Router::new().with_transport_logger(logger.clone());
//!
//! // FastClient (client-side): records outbound fast-path calls
//! let client = FastClient::connect("/tmp/agent.sock").await?
//!     .with_logger(logger);
//! ```

use std::path::{Path, PathBuf};

use serde::Serialize;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Which transport protocol a [`LogEntry`] was recorded on.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TransportKind {
    /// Fast binary path over a Unix domain socket (bincode frames).
    Fast,
    /// A2A-compatible JSON-RPC 2.0 over HTTP.
    A2a,
}

/// Direction of the logged operation from the local process's perspective.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// A task request was received from a remote caller (server side).
    Inbound,
    /// A task request was sent to a remote agent (client side).
    Outbound,
}

/// A single transport audit event.
///
/// Serialised as one NDJSON line in the log file. All fields are `pub` so
/// that callers can construct custom entries if needed.
#[derive(Debug, Serialize)]
pub struct LogEntry {
    /// ISO-8601 UTC timestamp of the event (microsecond precision).
    pub ts: String,
    /// Which transport was used.
    pub transport: TransportKind,
    /// Whether this is a server-side (inbound) or client-side (outbound) call.
    pub direction: Direction,
    /// Task ID copied from the request or response.
    pub task_id: String,
    /// Optional conversation/session identifier copied from the task request.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Full round-trip elapsed time in microseconds (transport + any remote
    /// processing such as LLM inference).
    ///
    /// For outbound (client) calls this is the full round-trip duration.
    /// For inbound (server) calls this is the agent handler duration.
    pub duration_us: u64,
    /// Time spent on LLM inference inside the remote agent, in microseconds.
    ///
    /// Populated only when the agent returns `llm_us` or `llm_ms` in its
    /// response metadata (e.g. agents backed by Ollama). Absent for
    /// pure-compute agents and all inbound entries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub llm_us: Option<u64>,
    /// Estimated non-LLM overhead in microseconds: `duration_us - llm_us`.
    ///
    /// Populated only when `llm_us` is available. This includes transport,
    /// serialization, routing, and other small caller/handler costs that are
    /// outside the measured LLM call.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport_us: Option<u64>,
    /// `"ok"` when the operation succeeded, `"error"` otherwise.
    pub status: &'static str,
    /// Human-readable error description. Present only when `status == "error"`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Serialised payload size in bytes, when measurable.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub payload_bytes: Option<usize>,
}

// ---------------------------------------------------------------------------
// TransportLogger
// ---------------------------------------------------------------------------

/// Async, non-blocking transport audit logger.
///
/// Obtain via [`TransportLogger::builder`]. All clones share the same
/// background writer task — cloning is cheap (an [`mpsc::Sender`] clone).
///
/// See the [module-level documentation](self) for usage examples.
#[derive(Debug, Clone)]
pub struct TransportLogger {
    tx: mpsc::Sender<LogEntry>,
}

impl TransportLogger {
    /// Returns a builder for configuring a new logger.
    pub fn builder() -> TransportLoggerBuilder {
        TransportLoggerBuilder::default()
    }

    /// Enqueue a log entry for writing.
    ///
    /// **Non-blocking**: if the internal channel is full the entry is silently
    /// dropped so that the transport hot path is never delayed.
    #[inline]
    pub fn record(&self, entry: LogEntry) {
        let _ = self.tx.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// Builder
// ---------------------------------------------------------------------------

/// Builder for [`TransportLogger`].
///
/// Obtain via [`TransportLogger::builder`].
pub struct TransportLoggerBuilder {
    log_dir: Option<PathBuf>,
    max_entries: usize,
    channel_capacity: usize,
}

impl Default for TransportLoggerBuilder {
    fn default() -> Self {
        Self {
            log_dir: None,
            max_entries: 1_000,
            channel_capacity: 4_096,
        }
    }
}

impl TransportLoggerBuilder {
    /// Directory where log files will be created.
    ///
    /// The directory is created with [`tokio::fs::create_dir_all`] if it does
    /// not already exist. **Required** — [`build`](Self::build) returns an
    /// error if omitted.
    #[must_use]
    pub fn log_dir(mut self, path: impl AsRef<Path>) -> Self {
        self.log_dir = Some(path.as_ref().to_path_buf());
        self
    }

    /// Maximum number of entries written to a single file before a new one is
    /// opened (default: **1 000**).
    #[must_use]
    pub fn max_entries(mut self, n: usize) -> Self {
        self.max_entries = n.max(1);
        self
    }

    /// Capacity of the in-memory channel between the transport threads and the
    /// writer task (default: **4 096**).
    ///
    /// When the writer falls behind and the channel fills, new entries are
    /// **dropped silently** to avoid blocking the transport.
    #[must_use]
    pub fn channel_capacity(mut self, n: usize) -> Self {
        self.channel_capacity = n.max(1);
        self
    }

    /// Spawn the background writer task and return the logger handle.
    ///
    /// # Errors
    ///
    /// Returns [`std::io::Error`] if `log_dir` was not configured or cannot
    /// be created.
    pub async fn build(self) -> Result<TransportLogger, std::io::Error> {
        let log_dir = self.log_dir.ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "log_dir not set")
        })?;

        tokio::fs::create_dir_all(&log_dir).await?;

        let (tx, rx) = mpsc::channel(self.channel_capacity);
        tokio::spawn(writer_task(rx, log_dir, self.max_entries));

        Ok(TransportLogger { tx })
    }
}

// ---------------------------------------------------------------------------
// Background writer task
// ---------------------------------------------------------------------------

async fn writer_task(mut rx: mpsc::Receiver<LogEntry>, log_dir: PathBuf, max_entries: usize) {
    let mut count: usize = 0;
    let mut file: Option<tokio::fs::File> = None;

    while let Some(entry) = rx.recv().await {
        // Open a new file on first entry or after rotation threshold.
        if file.is_none() || count >= max_entries {
            match open_log_file(&log_dir).await {
                Ok(f) => {
                    file = Some(f);
                    count = 0;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "transport-log: failed to open log file, dropping entry");
                    continue;
                }
            }
        }

        if let Some(ref mut f) = file {
            match serde_json::to_string(&entry) {
                Ok(mut line) => {
                    line.push('\n');
                    if let Err(e) = f.write_all(line.as_bytes()).await {
                        tracing::warn!(error = %e, "transport-log: write failed, will reopen on next entry");
                        file = None; // force re-open
                    } else {
                        count += 1;
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "transport-log: entry serialization failed");
                }
            }
        }
    }

    // Flush when all senders are dropped (graceful shutdown).
    if let Some(mut f) = file {
        let _ = f.flush().await;
    }
}

async fn open_log_file(log_dir: &Path) -> Result<tokio::fs::File, std::io::Error> {
    let path = log_dir.join(format!("transport-{}.ndjson", now_filename_safe()));
    tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .await
}

// ---------------------------------------------------------------------------
// Timestamp helpers — no external dependency required
// ---------------------------------------------------------------------------

/// Returns the current UTC instant as ISO-8601 with microsecond precision,
/// e.g. `2026-06-08T09:51:00.123456Z`.
pub fn now_iso8601() -> String {
    let (y, mo, d, h, mi, s, us) = now_parts();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{us:06}Z")
}

/// Filename-safe variant of [`now_iso8601`], e.g. `2026-06-08T09-51-00.123456Z`.
fn now_filename_safe() -> String {
    let (y, mo, d, h, mi, s, us) = now_parts();
    format!("{y:04}-{mo:02}-{d:02}T{h:02}-{mi:02}-{s:02}.{us:06}Z")
}

fn now_parts() -> (u64, u64, u64, u64, u64, u64, u32) {
    use std::time::{SystemTime, UNIX_EPOCH};
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = dur.as_secs();
    let us = dur.subsec_micros();
    let time_secs = total_secs % 86_400;
    let h = time_secs / 3_600;
    let mi = (time_secs % 3_600) / 60;
    let s = time_secs % 60;
    let (y, mo, d) = days_to_ymd(total_secs / 86_400);
    (y, mo, d, h, mi, s, us)
}

/// Gregorian calendar conversion from days since the Unix epoch.
///
/// Uses Howard Hinnant's algorithm:
/// <http://howardhinnant.github.io/date_algorithms.html>
fn days_to_ymd(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097; // day of era [0, 146096]
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365; // year of era [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // day of year [0, 365]
    let mp = (5 * doy + 2) / 153; // month index [0, 11]
    let d = doy - (153 * mp + 2) / 5 + 1; // day of month [1, 31]
    let mo = if mp < 10 { mp + 3 } else { mp - 9 }; // month [1, 12]
    let year = if mo <= 2 { y + 1 } else { y };
    (year, mo, d)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso8601_unix_epoch() {
        // 1970-01-01T00:00:00.000000Z
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
    }

    #[test]
    fn iso8601_known_date() {
        // 2026-06-08 = 20612 days since epoch
        assert_eq!(days_to_ymd(20_612), (2026, 6, 8));
    }

    #[test]
    fn iso8601_leap_day() {
        // 2000-02-29 = 11016 days since epoch
        assert_eq!(days_to_ymd(11_016), (2000, 2, 29));
    }

    #[test]
    fn log_entry_serializes_without_optional_fields() {
        let entry = LogEntry {
            ts: "2026-06-08T09:51:00.000000Z".into(),
            transport: TransportKind::Fast,
            direction: Direction::Outbound,
            task_id: "abc".into(),
            session_id: None,
            duration_us: 100,
            llm_us: None,
            transport_us: None,
            status: "ok",
            error: None,
            payload_bytes: None,
        };
        let json = serde_json::to_string(&entry).unwrap();
        assert!(!json.contains("error"));
        assert!(!json.contains("payload_bytes"));
        assert!(json.contains("\"fast\""));
        assert!(json.contains("\"outbound\""));
    }
}
