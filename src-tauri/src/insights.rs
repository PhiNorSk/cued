//! Background writer for listening-insights events (M9).
//!
//! The poll loop classifies user skips/seeks (see [`crate::automation`]) but
//! must never block on I/O. This module owns a bounded channel and a single
//! background task that drains it into the shared preset database. The loop
//! only ever calls [`InsightsSink::record`], which enqueues without waiting;
//! a full queue or a failed write is logged and dropped — playback control is
//! never disturbed.

use tauri::{AppHandle, Manager};
use tokio::sync::mpsc;

use crate::presets::{InsightWrite, PresetDb};

/// Upper bound on queued-but-unwritten events. Writes are tiny and rare
/// (only on genuine user actions), so this is generous; if it ever fills,
/// dropping is the correct, non-blocking failure mode.
const QUEUE_CAPACITY: usize = 256;

/// Enqueue-only handle held in Tauri managed state. Sending never blocks the
/// caller (the poll loop).
pub struct InsightsSink {
    tx: mpsc::Sender<InsightWrite>,
}

impl InsightsSink {
    /// Queue one event for the background writer. Non-blocking: a full queue
    /// (writer stalled) drops the event with a log line rather than delaying
    /// the poll loop.
    pub fn record(&self, write: InsightWrite) {
        if let Err(e) = self.tx.try_send(write) {
            eprintln!("cued: dropped a listening-insights event: {e}");
        }
    }
}

/// Start the background writer task and return the enqueue handle. Must be
/// called after [`PresetDb`] is in managed state (the writer reads it).
pub fn spawn(app: &AppHandle) -> InsightsSink {
    let (tx, mut rx) = mpsc::channel::<InsightWrite>(QUEUE_CAPACITY);
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        while let Some(write) = rx.recv().await {
            match app.state::<PresetDb>().store() {
                Ok(store) => {
                    if let Err(e) = store.record_event(&write) {
                        eprintln!("cued: failed to record a listening-insights event: {e}");
                    }
                }
                Err(e) => {
                    eprintln!("cued: insights store unavailable, dropping event: {e}");
                }
            }
        }
    });
    InsightsSink { tx }
}
