//! Live iRacing telemetry via the `kerb` crate, on a dedicated thread.
//!
//! kerb's connection is `!Send`, so the thread creates it and forwards events through a
//! bounded queue. The thread covers what kerb 0.4.0 leaves open:
//! - `connect` also succeeds on a stale mapping that other tools (iRacingUI, other
//!   overlays) keep open after the sim closed, so the sim only counts as live once
//!   `SessionTick` moves;
//! - it waits on `Local\IRSDKDataValidEventName`, but iRacing creates
//!   `Local\IRSDKDataValidEvent`, so on its own it polls every 16 ms. The thread waits on
//!   the real event itself and asks kerb for the frame without waiting. kerb also hands back
//!   the last frame again when the sim froze or crashed; with no staleness timeout of its
//!   own, the thread gives up after 5 s without a new tick;
//! - its strict YAML parse fails in public lobbies (`UserName: *Speedy`), so
//!   [`parse_session_yaml`] line-scans the raw session text instead.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use super::{SessionInfo, TelemetryEvent, parse_degrees, parse_track_length};

/// Events the queue holds before frames are dropped: about 17 s at 60 Hz.
const QUEUE_LEN: usize = 1024;

/// Reads iRacing on a background thread and queues [`TelemetryEvent`]s.
pub struct IracingReader {
    rx: Receiver<TelemetryEvent>,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

/// What the app changes while the thread runs.
#[derive(Debug)]
struct Shared {
    stop: AtomicBool,
    wake_divisor: AtomicU32,
}

impl IracingReader {
    /// Starts the reader thread. `wake` is called after events are queued (the app
    /// uses it to request a repaint), at most once per telemetry frame.
    pub fn spawn(wake: Box<dyn Fn() + Send + Sync>) -> Self {
        let (tx, rx) = sync_channel(QUEUE_LEN);
        let shared = Arc::new(Shared { stop: AtomicBool::new(false), wake_divisor: AtomicU32::new(1) });
        let thread = start_thread(tx, Arc::clone(&shared), wake);
        Self { rx, shared, thread }
    }

    /// Events queued since the last call, oldest first.
    pub fn drain(&self) -> impl Iterator<Item = TelemetryEvent> + '_ {
        self.rx.try_iter()
    }

    /// Wake the UI on every `n`th frame (1 = 60 Hz, 2 = 30 Hz).
    pub fn set_wake_divisor(&self, n: u32) {
        self.shared.wake_divisor.store(n.max(1), Ordering::Relaxed);
    }

    /// Stops the thread (also done on drop).
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            if thread.join().is_err() {
                log::error!("the iRacing reader thread panicked");
            }
        }
    }
}

impl Drop for IracingReader {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(windows)]
fn start_thread(
    tx: SyncSender<TelemetryEvent>,
    shared: Arc<Shared>,
    wake: Box<dyn Fn() + Send + Sync>,
) -> Option<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("iracing-telemetry".into())
        .spawn(move || live::run(&mut live::Outbox::new(tx, shared, wake)))
        .map_err(|e| log::error!("couldn't start the iRacing reader thread: {e}"))
        .ok()
}

/// iRacing only runs on Windows; elsewhere the reader never produces events.
#[cfg(not(windows))]
fn start_thread(
    _tx: SyncSender<TelemetryEvent>,
    _shared: Arc<Shared>,
    _wake: Box<dyn Fn() + Send + Sync>,
) -> Option<JoinHandle<()>> {
    None
}

/// Extracts the overlay's [`SessionInfo`] from iRacing's session-info YAML.
///
/// A line scan of the `WeekendInfo` and `DriverInfo` sections rather than a YAML parse:
/// iRacing doesn't quote user text, so one driver named `*Speedy` makes a strict parser
/// reject the whole document. The player's car is the `Drivers` entry whose `CarIdx` is
/// `DriverCarIdx`, which isn't its position in the list.
pub fn parse_session_yaml(yaml: &str) -> SessionInfo {
    let mut scan = YamlScan::default();
    for line in yaml.trim_start_matches('\u{feff}').lines() {
        if let Some(entry) = Entry::parse(line) {
            scan.add(&entry);
        }
    }
    scan.finish()
}

/// One `key: value` line.
#[derive(Debug)]
struct Entry<'a> {
    indent: usize,
    /// Starts a list item (`- key: value`).
    item: bool,
    key: &'a str,
    value: &'a str,
}

impl<'a> Entry<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let body = line.trim_start_matches(' ');
        let indent = line.len() - body.len();
        let (item, body) = match body.strip_prefix("- ") {
            Some(rest) => (true, rest),
            None => (false, body),
        };
        let (key, value) = body.split_once(':')?;
        let is_key = !key.is_empty() && !key.contains(char::is_whitespace);
        is_key.then(|| Self { indent, item, key, value: value.trim() })
    }
}

/// The top-level sections the scan reads.
#[derive(Debug, Default, Clone, Copy)]
enum Section {
    #[default]
    Other,
    Weekend,
    Drivers,
}

/// A `DriverInfo.Drivers` entry.
#[derive(Debug, Default)]
struct Driver<'a> {
    car_idx: Option<&'a str>,
    car_name: Option<&'a str>,
    car_short_name: Option<&'a str>,
}

/// Raw values collected by [`parse_session_yaml`].
#[derive(Debug, Default)]
struct YamlScan<'a> {
    section: Section,
    /// Indent of the current section's own keys.
    key_indent: Option<usize>,
    /// Inside `DriverInfo.Drivers`, not another `DriverInfo` list such as `DriverTires`.
    in_drivers: bool,
    track_name: Option<&'a str>,
    track_display_name: Option<&'a str>,
    track_config_name: Option<&'a str>,
    track_length: Option<&'a str>,
    latitude: Option<&'a str>,
    longitude: Option<&'a str>,
    player_car_idx: Option<&'a str>,
    drivers: Vec<Driver<'a>>,
}

impl<'a> YamlScan<'a> {
    fn add(&mut self, e: &Entry<'a>) {
        if e.indent == 0 && !e.item {
            self.section = match e.key {
                "WeekendInfo" => Section::Weekend,
                "DriverInfo" => Section::Drivers,
                _ => Section::Other,
            };
            self.key_indent = None;
            self.in_drivers = false;
            return;
        }
        let own_key = !e.item && e.indent == *self.key_indent.get_or_insert(e.indent);
        match self.section {
            Section::Weekend if own_key => self.add_weekend(e),
            Section::Drivers if own_key => {
                self.in_drivers = e.key == "Drivers";
                if e.key == "DriverCarIdx" {
                    self.player_car_idx = Some(e.value);
                }
            }
            Section::Drivers if self.in_drivers => self.add_driver(e),
            _ => {}
        }
    }

    fn add_weekend(&mut self, e: &Entry<'a>) {
        let field = match e.key {
            "TrackName" => &mut self.track_name,
            "TrackDisplayName" => &mut self.track_display_name,
            "TrackConfigName" => &mut self.track_config_name,
            "TrackLength" => &mut self.track_length,
            "TrackLatitude" => &mut self.latitude,
            "TrackLongitude" => &mut self.longitude,
            _ => return,
        };
        *field = Some(e.value);
    }

    fn add_driver(&mut self, e: &Entry<'a>) {
        if e.item || self.drivers.is_empty() {
            self.drivers.push(Driver::default());
        }
        let Some(driver) = self.drivers.last_mut() else { return };
        let field = match e.key {
            "CarIdx" => &mut driver.car_idx,
            "CarScreenName" => &mut driver.car_name,
            "CarScreenNameShort" => &mut driver.car_short_name,
            _ => return,
        };
        *field = Some(e.value);
    }

    fn finish(self) -> SessionInfo {
        let car_idx = |raw: Option<&str>| raw.and_then(scalar)?.parse::<i64>().ok();
        let player = car_idx(self.player_car_idx);
        let car = player.and_then(|p| self.drivers.iter().find(|d| car_idx(d.car_idx) == Some(p)));
        let degrees = |raw: Option<&str>| raw.and_then(scalar).as_deref().and_then(parse_degrees);
        SessionInfo {
            track_name: self.track_name.and_then(scalar),
            track_display_name: self.track_display_name.and_then(scalar),
            track_config_name: self.track_config_name.and_then(scalar),
            track_length_m: self.track_length.and_then(scalar).as_deref().and_then(parse_track_length),
            track_latlon: degrees(self.latitude).zip(degrees(self.longitude)),
            car_name: car.and_then(|d| d.car_name).and_then(scalar),
            car_short_name: car.and_then(|d| d.car_short_name).and_then(scalar),
        }
    }
}

/// A plain or quoted YAML scalar; `None` when empty or `~`.
fn scalar(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let value = if let Some(inner) = raw.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
        unescape_double_quoted(inner)
    } else if let Some(inner) = raw.strip_prefix('\'').and_then(|s| s.strip_suffix('\'')) {
        inner.replace("''", "'")
    } else {
        raw.to_string()
    };
    let value = value.trim();
    (!value.is_empty() && value != "~").then(|| value.to_string())
}

/// Resolves `\"` and `\\`, the escapes that occur in names.
fn unescape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        out.push(if c == '\\' { chars.next().unwrap_or('\\') } else { c });
    }
    out
}

/// The reader thread.
#[cfg(windows)]
mod live {
    use std::collections::VecDeque;
    use std::sync::Arc;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc::{SyncSender, TrySendError};
    use std::time::{Duration, Instant};

    use kerb::iracing::{IRsdkConnection, IracingFrame};
    use kerb::{Connection, ReadResult, SimConnection, SimType};
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{OpenEventW, WaitForSingleObject};

    use super::{Shared, parse_session_yaml};
    use crate::telemetry::{SessionInfo, TelemetryEvent, TelemetryFrame};

    /// Longest wait in `read_frame`, which bounds how long `stop` takes.
    const READ_TIMEOUT_MS: u32 = 100;
    /// Pause between connection attempts.
    const RETRY_DELAY: Duration = Duration::from_secs(1);
    /// No new `SessionTick` for this long and the sim counts as gone.
    const STALE_AFTER: Duration = Duration::from_secs(5);
    /// `irsdk_TrkLoc` value of `PlayerTrackSurface` in the pit stall.
    const TRK_LOC_IN_PIT_STALL: i32 = 1;

    /// Connects, reads until the sim goes away, backs off and retries until stopped.
    pub(super) fn run(out: &mut Outbox) {
        let mut last_error = None;
        while !out.stopped() {
            out.flush();
            out.wake_app();
            match SimConnection::connect_to(SimType::IRacing) {
                Ok(Connection::IRacing(conn)) => {
                    last_error = None;
                    read_connection(&conn, out);
                }
                Ok(_) => {}
                Err(e) => {
                    let e = e.to_string();
                    if last_error.as_ref() != Some(&e) {
                        log::debug!("waiting for iRacing: {e}");
                        last_error = Some(e);
                    }
                }
            }
            out.pause(RETRY_DELAY);
        }
    }

    /// Reads one connection until the sim closes, goes stale, or the reader stops.
    fn read_connection(conn: &IRsdkConnection, out: &mut Outbox) {
        let mut ticks = TickWatch::new(Instant::now());
        let mut session = SessionWatch::default();
        let mut event = None;
        let ended = loop {
            if out.stopped() {
                break "reader stopped";
            }
            // The sim creates the event when it starts broadcasting.
            if event.is_none() {
                event = DataValidEvent::open();
            }
            let result = match &event {
                Some(ev) => {
                    ev.wait(READ_TIMEOUT_MS);
                    conn.read_frame(0)
                }
                None => conn.read_frame(READ_TIMEOUT_MS),
            };
            let now = Instant::now();
            match result {
                ReadResult::Frame(f) => match ticks.observe(f.session_tick, now) {
                    Tick::Repeat => {}
                    tick => {
                        if tick == Tick::Live {
                            log::info!("iRacing connected");
                            out.send(TelemetryEvent::Connected);
                        }
                        session.refresh(conn, out);
                        out.send(TelemetryEvent::Frame(telemetry_frame(&f)));
                    }
                },
                ReadResult::NotReady => {}
                ReadResult::Disconnected => break "sim closed",
            }
            out.wake_app();
            if ticks.is_stale(now) {
                break "no new data";
            }
        };
        if ticks.live {
            log::info!("iRacing disconnected ({ended})");
            out.send(TelemetryEvent::Disconnected);
            out.wake_app();
        }
    }

    /// iRacing's "new data" event, which it signals for every frame it publishes.
    struct DataValidEvent(HANDLE);

    impl DataValidEvent {
        fn open() -> Option<Self> {
            const SYNCHRONIZE: u32 = 0x0010_0000;
            let name: Vec<u16> = "Local\\IRSDKDataValidEvent\0".encode_utf16().collect();
            // SAFETY: `name` is a NUL-terminated UTF-16 string that outlives the call.
            let handle = unsafe { OpenEventW(SYNCHRONIZE, 0, name.as_ptr()) };
            (!handle.is_null()).then_some(Self(handle))
        }

        /// Waits up to `ms` for the next frame; `false` on timeout.
        fn wait(&self, ms: u32) -> bool {
            // SAFETY: the handle is valid until drop.
            unsafe { WaitForSingleObject(self.0, ms) == WAIT_OBJECT_0 }
        }
    }

    impl Drop for DataValidEvent {
        fn drop(&mut self) {
            // SAFETY: we own the handle and close it once.
            unsafe { CloseHandle(self.0) };
        }
    }

    fn telemetry_frame(f: &IracingFrame) -> TelemetryFrame {
        TelemetryFrame {
            session_time: f.session_time,
            lap_dist_pct: f64::from(f.lap_dist_pct),
            throttle: f.throttle,
            brake: f.brake,
            on_track: CarState::of(f).is_driving(),
        }
    }

    /// The frame fields that say whether the player is out driving.
    #[derive(Debug, Clone, Copy, PartialEq)]
    struct CarState {
        /// `IsOnTrack`.
        on_track: bool,
        /// `IsReplayPlaying`.
        replay: bool,
        /// `PlayerTrackSurface` (`irsdk_TrkLoc`: -1 not in world, 1 pit stall, 3 on track).
        surface: i32,
        /// `PlayerCarInPitStall`.
        in_pit_stall: bool,
        /// `IsInGarage` or `IsGarageVisible`.
        in_garage: bool,
    }

    impl CarState {
        fn of(f: &IracingFrame) -> Self {
            Self {
                on_track: f.is_on_track,
                replay: f.is_replay_playing,
                surface: f.player_track_surface,
                in_pit_stall: f.player_car_in_pit_stall,
                in_garage: f.is_in_garage || f.is_garage_visible,
            }
        }

        /// In the car and moving under their own inputs: not in the pit stall (where
        /// `Brake` reads 1.0 from the automatic hold), the garage or a replay.
        fn is_driving(self) -> bool {
            self.on_track
                && !self.replay
                && !self.in_garage
                && !self.in_pit_stall
                && self.surface >= 0
                && self.surface != TRK_LOC_IN_PIT_STALL
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Tick {
        /// Same tick as the previous frame, or the first one seen: nothing new.
        Repeat,
        /// The first tick that moved: the sim is live.
        Live,
        /// A further new tick.
        New,
    }

    /// Follows `SessionTick` on one connection.
    #[derive(Debug)]
    struct TickWatch {
        last: Option<i32>,
        live: bool,
        last_new: Instant,
    }

    impl TickWatch {
        fn new(now: Instant) -> Self {
            Self { last: None, live: false, last_new: now }
        }

        fn observe(&mut self, tick: i32, now: Instant) -> Tick {
            match self.last.replace(tick) {
                Some(prev) if prev != tick => {
                    self.last_new = now;
                    if std::mem::replace(&mut self.live, true) { Tick::New } else { Tick::Live }
                }
                _ => Tick::Repeat,
            }
        }

        fn is_stale(&self, now: Instant) -> bool {
            now.saturating_duration_since(self.last_new) > STALE_AFTER
        }
    }

    /// Re-reads the session YAML when iRacing bumps `SessionInfoUpdate` and sends the
    /// result when it differs from what this connection sent last. (iRacing bumps it for
    /// every change to results, which the overlay doesn't use.)
    #[derive(Debug, Default)]
    struct SessionWatch {
        version: Option<i32>,
        sent: Option<SessionInfo>,
    }

    impl SessionWatch {
        fn refresh(&mut self, conn: &IRsdkConnection, out: &mut Outbox) {
            let version = conn.session_info_update();
            // Recorded before parsing, so a document without usable data is scanned once.
            if self.version.replace(version) == Some(version) {
                return;
            }
            if let Some(info) = conn.session_yaml().map(|yaml| parse_session_yaml(&yaml))
                && self.sent.as_ref() != Some(&info)
            {
                log::info!(
                    "iRacing session: {} / {}",
                    info.full_track_name().as_deref().unwrap_or("unknown track"),
                    info.car_name.as_deref().unwrap_or("unknown car"),
                );
                self.sent = Some(info.clone());
                out.send(TelemetryEvent::Session(info));
            }
        }
    }

    /// The reader thread's end of the queue. Never blocks: when the app falls behind,
    /// frames are dropped, while status events wait in a backlog so the app still learns
    /// about every connect, disconnect and session change, in order.
    pub(super) struct Outbox {
        tx: SyncSender<TelemetryEvent>,
        shared: Arc<Shared>,
        wake: Box<dyn Fn() + Send + Sync>,
        backlog: VecDeque<TelemetryEvent>,
        /// Frames queued, for the wake divisor.
        frames: u32,
        /// Something the app should be woken for was queued since the last wake.
        wake_pending: bool,
    }

    impl Outbox {
        pub(super) fn new(
            tx: SyncSender<TelemetryEvent>,
            shared: Arc<Shared>,
            wake: Box<dyn Fn() + Send + Sync>,
        ) -> Self {
            Self { tx, shared, wake, backlog: VecDeque::new(), frames: 0, wake_pending: false }
        }

        fn stopped(&self) -> bool {
            self.shared.stop.load(Ordering::Relaxed)
        }

        /// Queues an event; [`Self::wake_app`] then wakes the app for it.
        fn send(&mut self, event: TelemetryEvent) {
            if !matches!(event, TelemetryEvent::Frame(_)) {
                self.backlog.push_back(event);
                self.flush();
            } else if self.flush() && self.tx.try_send(event).is_ok() {
                self.frames = self.frames.wrapping_add(1);
                let divisor = self.shared.wake_divisor.load(Ordering::Relaxed).max(1);
                self.wake_pending |= self.frames.is_multiple_of(divisor);
            }
        }

        /// Queues the backlog; true once it's empty.
        fn flush(&mut self) -> bool {
            while let Some(event) = self.backlog.pop_front() {
                match self.tx.try_send(event) {
                    Ok(()) => self.wake_pending = true,
                    Err(TrySendError::Full(event) | TrySendError::Disconnected(event)) => {
                        self.backlog.push_front(event);
                        return false;
                    }
                }
            }
            true
        }

        /// Calls `wake` if a status event or an `n`th frame was queued since the last call.
        /// Called once per telemetry frame, however many events it produced.
        fn wake_app(&mut self) {
            if std::mem::take(&mut self.wake_pending) {
                (self.wake)();
            }
        }

        /// Sleeps for `d`, or less when the reader is stopped.
        fn pause(&self, d: Duration) {
            let deadline = Instant::now() + d;
            while !self.stopped() {
                let left = deadline.saturating_duration_since(Instant::now());
                if left.is_zero() {
                    break;
                }
                std::thread::park_timeout(left);
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize};
        use std::sync::mpsc::{Receiver, sync_channel};

        use super::*;

        fn frame(brake: f32) -> TelemetryEvent {
            TelemetryEvent::Frame(TelemetryFrame {
                session_time: 1.0,
                lap_dist_pct: 0.5,
                throttle: 0.0,
                brake,
                on_track: true,
            })
        }

        fn outbox(capacity: usize, divisor: u32) -> (Outbox, Receiver<TelemetryEvent>, Arc<AtomicUsize>) {
            let (tx, rx) = sync_channel(capacity);
            let shared = Arc::new(Shared { stop: AtomicBool::new(false), wake_divisor: AtomicU32::new(divisor) });
            let wakes = Arc::new(AtomicUsize::new(0));
            let counter = Arc::clone(&wakes);
            let wake = Box::new(move || {
                counter.fetch_add(1, Ordering::Relaxed);
            });
            (Outbox::new(tx, shared, wake), rx, wakes)
        }

        #[test]
        fn wakes_once_per_frame_on_status_events_and_every_nth_frame() {
            let (mut out, rx, wakes) = outbox(64, 3);
            let wakes = move || wakes.load(Ordering::Relaxed);
            out.send(TelemetryEvent::Connected);
            out.send(TelemetryEvent::Session(SessionInfo::default()));
            out.send(frame(0.0));
            out.wake_app();
            assert_eq!(wakes(), 1, "first frame with its status events");
            for _ in 0..8 {
                out.send(frame(0.0));
                out.wake_app();
            }
            assert_eq!(wakes(), 1 + 3, "frames 3, 6 and 9");
            out.send(TelemetryEvent::Disconnected);
            out.wake_app();
            out.wake_app();
            assert_eq!(wakes(), 5);
            assert_eq!(rx.try_iter().count(), 12);
        }

        #[test]
        fn full_queue_drops_frames_but_keeps_status_events_in_order() {
            let (mut out, rx, _) = outbox(2, 1);
            out.send(TelemetryEvent::Connected);
            out.send(frame(0.1));
            out.send(frame(0.2)); // dropped: queue full
            out.send(TelemetryEvent::Disconnected); // backlogged
            out.send(frame(0.3)); // dropped: backlog first
            assert_eq!(rx.try_iter().collect::<Vec<_>>(), [TelemetryEvent::Connected, frame(0.1)]);
            out.send(frame(0.4));
            assert_eq!(rx.try_iter().collect::<Vec<_>>(), [TelemetryEvent::Disconnected, frame(0.4)]);
        }

        #[test]
        fn backlog_drains_while_idle() {
            let (mut out, rx, _) = outbox(1, 1);
            out.send(TelemetryEvent::Connected);
            out.send(TelemetryEvent::Disconnected);
            assert_eq!(rx.try_recv(), Ok(TelemetryEvent::Connected));
            assert!(out.flush());
            assert_eq!(rx.try_recv(), Ok(TelemetryEvent::Disconnected));
        }

        #[test]
        fn pause_returns_early_when_stopped() {
            let (out, _rx, _) = outbox(1, 1);
            out.shared.stop.store(true, Ordering::Relaxed);
            let start = Instant::now();
            out.pause(Duration::from_secs(5));
            assert!(start.elapsed() < Duration::from_millis(100));
        }

        #[test]
        fn live_only_once_the_tick_moves() {
            let t0 = Instant::now();
            let mut w = TickWatch::new(t0);
            assert_eq!(w.observe(500, t0), Tick::Repeat, "first frame may be a stale mapping");
            assert_eq!(w.observe(500, t0 + Duration::from_millis(16)), Tick::Repeat);
            assert!(!w.live);
            assert_eq!(w.observe(501, t0 + Duration::from_millis(33)), Tick::Live);
            assert_eq!(w.observe(502, t0 + Duration::from_millis(50)), Tick::New);
            assert_eq!(w.observe(502, t0 + Duration::from_millis(66)), Tick::Repeat);
            assert_eq!(w.observe(1, t0 + Duration::from_millis(83)), Tick::New, "sim restarted its count");
        }

        #[test]
        fn stale_after_five_seconds_without_a_new_tick() {
            let t0 = Instant::now();
            let mut w = TickWatch::new(t0);
            w.observe(1, t0);
            assert!(!w.is_stale(t0 + Duration::from_secs(4)));
            assert!(w.is_stale(t0 + Duration::from_millis(5001)), "a mapping that never ticks goes stale");
            w.observe(2, t0 + Duration::from_secs(4));
            for ms in (4000..8900).step_by(16) {
                w.observe(2, t0 + Duration::from_millis(ms)); // repeated frames don't count
            }
            assert!(!w.is_stale(t0 + Duration::from_millis(8900)));
            assert!(w.is_stale(t0 + Duration::from_millis(9001)));
        }

        #[test]
        fn driving_excludes_pit_stall_garage_and_replays() {
            let driving = CarState { on_track: true, replay: false, surface: 3, in_pit_stall: false, in_garage: false };
            assert!(driving.is_driving());
            assert!(CarState { surface: 2, ..driving }.is_driving(), "approaching or on pit road");
            assert!(CarState { surface: 0, ..driving }.is_driving(), "off track");
            assert!(!CarState { surface: TRK_LOC_IN_PIT_STALL, ..driving }.is_driving());
            assert!(!CarState { in_pit_stall: true, ..driving }.is_driving());
            assert!(!CarState { surface: -1, ..driving }.is_driving());
            assert!(!CarState { on_track: false, ..driving }.is_driving());
            assert!(!CarState { replay: true, ..driving }.is_driving());
            assert!(!CarState { in_garage: true, ..driving }.is_driving());
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::*;

    /// WeekendInfo the way iRacing writes it, trimmed; `{config}` is replaced per test.
    const WEEKEND: &str = "---
WeekendInfo:
 Encoding: ISO_8859_1
 TrackName: nurburgring combined
 TrackID: 999
 TrackLength: 25.3781 km
 TrackLengthOfficial: 25.38 km
 TrackDisplayName: Nürburgring Combined
 TrackDisplayShortName: Nürburgring
 TrackConfigName: {config}
 TrackCity: Nürburg
 TrackCountry: Germany
 TrackLatitude: 50.335618 m
 TrackLongitude: 6.947517 m
 TrackNumTurns: 170
 WeekendOptions:
  NumStarters: 60
  TimeOfDay: 3:30 pm
 TelemetryOptions:
  TelemetryDiskFile: \"\"

SessionInfo:
 CurrentSessionNum: 0
 Sessions:
 - SessionNum: 0
   SessionType: Practice
   ResultsPositions:
   - Position: 1
     CarIdx: 1
     Lap: 3
   - Position: 2
     CarIdx: 7
     Lap: 2
";

    /// DriverInfo with the player (`CarIdx` 7) third in the list, after a driver whose
    /// unquoted name is invalid YAML.
    const DRIVERS: &str = "
CameraInfo:
 Groups:
 - GroupNum: 1
   GroupName: Nose
DriverInfo:
 DriverCarIdx: 7
 DriverUserID: 1000
 PaceCarIdx: 0
 DriverSetupName: baseline.sto
 DriverTires:
 - TireIndex: 0
   TireCompoundType: \"Hard\"
 Drivers:
 - CarIdx: 0
   UserName: Pace Car
   CarScreenName: Mercedes-AMG GT Safety Car
   CarScreenNameShort: Mercedes-AMG GT SC
 - CarIdx: 1
   UserName: *Speedy
   AbbrevName: *Speedy
   CarScreenName: Porsche 911 GT3 R (992)
   CarScreenNameShort: Porsche 992 GT3 R
 - CarIdx: 7
   UserName: Jörg Beispiel
   TeamName: 'Équipe d''Essai'
   CarScreenName: \"Ferrari 296 GT3\"
   CarScreenNameShort: Ferrari 296
   CarCfgName:
SplitTimeInfo:
 Sectors:
 - SectorNum: 0
   SectorStartPct: 0.000000
CarSetup:
 CarIdx: 1
 CarScreenName: not a driver
";

    fn session_yaml(config: &str) -> String {
        format!("{}{DRIVERS}", WEEKEND.replace("{config}", config))
    }

    #[test]
    fn extracts_track_and_player_car() {
        let info = parse_session_yaml(&session_yaml("Nordschleife + Grand Prix"));
        assert_eq!(
            info,
            SessionInfo {
                track_name: Some("nurburgring combined".into()),
                track_display_name: Some("Nürburgring Combined".into()),
                track_config_name: Some("Nordschleife + Grand Prix".into()),
                track_length_m: Some(25378.1),
                track_latlon: Some((50.335618, 6.947517)),
                car_name: Some("Ferrari 296 GT3".into()),
                car_short_name: Some("Ferrari 296".into()),
            }
        );
    }

    #[test]
    fn empty_or_null_track_config_is_none() {
        for config in ["", "~", "\"\""] {
            let info = parse_session_yaml(&session_yaml(config));
            assert_eq!(info.track_config_name, None, "{config:?}");
            assert_eq!(info.full_track_name().as_deref(), Some("Nürburgring Combined"));
        }
    }

    #[test]
    fn handles_crlf_line_endings() {
        let yaml = session_yaml("Nordschleife").replace('\n', "\r\n");
        let info = parse_session_yaml(&yaml);
        assert_eq!(info.track_config_name.as_deref(), Some("Nordschleife"));
        assert_eq!(info.track_length_m, Some(25378.1));
        assert_eq!(info.car_short_name.as_deref(), Some("Ferrari 296"));
    }

    #[test]
    fn player_car_found_by_car_idx_not_list_position() {
        let yaml = session_yaml("x").replace("DriverCarIdx: 7", "DriverCarIdx: 1");
        assert_eq!(parse_session_yaml(&yaml).car_name.as_deref(), Some("Porsche 911 GT3 R (992)"));
        let yaml = session_yaml("x").replace("DriverCarIdx: 7", "DriverCarIdx: 2");
        assert_eq!(parse_session_yaml(&yaml).car_name, None, "no Drivers entry for CarIdx 2");
    }

    #[test]
    fn ignores_car_keys_outside_the_drivers_list() {
        // ResultsPositions (CarIdx 7) and CarSetup carry CarIdx/CarScreenName keys too.
        let yaml = session_yaml("x").replace(" - CarIdx: 7\n", " - CarIdx: 8\n");
        let info = parse_session_yaml(&yaml);
        assert_eq!(info.car_name, None);
        assert_eq!(info.track_display_name.as_deref(), Some("Nürburgring Combined"));
    }

    #[test]
    fn missing_sections_leave_fields_empty() {
        assert_eq!(parse_session_yaml(""), SessionInfo::default());
        assert_eq!(parse_session_yaml("not yaml at all\n:::\n- - -"), SessionInfo::default());
        let weekend_only = parse_session_yaml(&WEEKEND.replace("{config}", ""));
        assert_eq!(weekend_only.track_name.as_deref(), Some("nurburgring combined"));
        assert_eq!(weekend_only.car_name, None);
    }

    #[test]
    fn unparseable_numbers_are_none() {
        let yaml = session_yaml("x")
            .replace("TrackLength: 25.3781 km", "TrackLength: long")
            .replace("TrackLatitude: 50.335618 m", "TrackLatitude:");
        let info = parse_session_yaml(&yaml);
        assert_eq!(info.track_length_m, None);
        assert_eq!(info.track_latlon, None);
    }

    #[test]
    fn unquotes_scalars() {
        assert_eq!(scalar(" plain text "), Some("plain text".into()));
        assert_eq!(scalar("\"a \\\"quoted\\\" name\""), Some("a \"quoted\" name".into()));
        assert_eq!(scalar("'it''s'"), Some("it's".into()));
        assert_eq!(scalar("*Speedy"), Some("*Speedy".into()));
        assert_eq!(scalar("~"), None);
        assert_eq!(scalar("''"), None);
        assert_eq!(scalar(""), None);
    }

    #[test]
    fn reader_stops_promptly() {
        let mut reader = IracingReader::spawn(Box::new(|| {}));
        reader.set_wake_divisor(0);
        std::thread::sleep(Duration::from_millis(50));
        let start = Instant::now();
        reader.stop();
        assert!(start.elapsed() < Duration::from_millis(300), "{:?}", start.elapsed());
        reader.stop();
    }
}
