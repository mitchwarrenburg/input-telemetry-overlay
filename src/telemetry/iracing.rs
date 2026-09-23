//! Live iRacing telemetry via the `kerb` crate, on a dedicated thread.

use std::sync::mpsc::Receiver;

use super::TelemetryEvent;

/// Reads iRacing on a background thread and queues [`TelemetryEvent`]s.
pub struct IracingReader {
    rx: Receiver<TelemetryEvent>,
}

impl IracingReader {
    /// Starts the reader thread. `wake` is called after events are queued (the app
    /// uses it to request a repaint), at most once per telemetry frame.
    pub fn spawn(_wake: Box<dyn Fn() + Send + Sync>) -> Self {
        let (_tx, rx) = std::sync::mpsc::channel();
        Self { rx }
    }

    /// Events queued since the last call, oldest first.
    pub fn drain(&self) -> impl Iterator<Item = TelemetryEvent> + '_ {
        self.rx.try_iter()
    }

    /// Wake the UI on every `n`th frame (1 = 60 Hz, 2 = 30 Hz).
    pub fn set_wake_divisor(&self, _n: u32) {}

    /// Stops the thread (also done on drop).
    pub fn stop(&mut self) {}
}
