//! A stand-in for iRacing: publishes telemetry through the sim's shared memory, laid out
//! the way iRacing does it, so the overlay can be tried and tested without the sim. It
//! drives the bundled Silverstone lap in the Ferrari 296 GT3 at 60 Hz, varying the inputs
//! from lap to lap like demo mode.
//!
//! ```text
//! cargo run --example fake_iracing -- [options]
//!   --secs N        stop after N seconds (default: when Enter is pressed)
//!   --pit N         sit in the pit stall for the first N seconds, brake held at 100 %
//!   --car NAME      the player's CarScreenName (default: Ferrari 296 GT3)
//!   --track NAME    TrackDisplayName (default: Silverstone Circuit); the length and
//!                   position stay Silverstone's
//!   --config NAME   TrackConfigName, "" for none (default: Arena Grand Prix)
//!   --kerb-event    also signal Local\IRSDKDataValidEventName, the event kerb 0.4.0 waits
//!                   on (iRacing only signals Local\IRSDKDataValidEvent)
//! ```
//!
//! Stopping with Enter or `--secs` clears the connected flag like iRacing does on exit;
//! Ctrl+C or killing the process leaves it set, like a crash. The session YAML is
//! republished every 10 s with new results, as the sim does during a session, and names
//! one driver `*Speedy`, which strict YAML parsers reject.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    match fake::run(std::env::args().skip(1)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fake_iracing: {e}");
            std::process::ExitCode::FAILURE
        }
    }
}

#[cfg(not(windows))]
fn main() {
    eprintln!("fake_iracing needs Windows: iRacing publishes telemetry through Windows shared memory");
}

#[cfg(windows)]
mod fake {
    use std::ffi::c_void;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering, fence};
    use std::time::{Duration, Instant};

    use ito::demo::{SimulatedDriver, sample_lap};
    use ito::telemetry::TelemetryFrame;

    const USAGE: &str =
        "usage: fake_iracing [--secs N] [--pit N] [--car NAME] [--track NAME] [--config NAME] [--kerb-event]";

    const MAP_NAME: &str = r"Local\IRSDKMemMapFileName";
    const EVENT_NAME: &str = r"Local\IRSDKDataValidEvent";
    const KERB_EVENT_NAME: &str = r"Local\IRSDKDataValidEventName";

    const TICK_RATE: i32 = 60;
    const MAP_SIZE: usize = 1 << 20;
    const VAR_HEADERS_AT: usize = 144;
    const VAR_HEADER_LEN: usize = 144;
    const SESSION_AT: usize = 16 * 1024;
    const SESSION_LEN: usize = 512 * 1024;
    const BUFFERS_AT: usize = SESSION_AT + SESSION_LEN;
    const BUFFER_STRIDE: usize = 4096;
    const NUM_BUF: usize = 3;

    /// `irsdk_header` field offsets.
    const H_VER: usize = 0;
    const H_STATUS: usize = 4;
    const H_TICK_RATE: usize = 8;
    const H_SESSION_UPDATE: usize = 12;
    const H_SESSION_LEN: usize = 16;
    const H_SESSION_OFFSET: usize = 20;
    const H_NUM_VARS: usize = 24;
    const H_VAR_HEADER_OFFSET: usize = 28;
    const H_NUM_BUF: usize = 32;
    const H_BUF_LEN: usize = 36;
    /// `varBuf[i]` is `{ tickCount, bufOffset, pad[2] }` from here.
    const H_VAR_BUF: usize = 48;

    const PLAYER_CAR_IDX: i32 = 3;
    const TRACK_LENGTH_M: f32 = 5796.1;
    /// Where the pit stall is, as `LapDistPct`.
    const PIT_STALL_PCT: f32 = 0.041938;
    /// `SessionTime` when the fake starts, so it isn't mistaken for a fresh session.
    const SESSION_TIME_AT_START: f64 = 600.0;
    const SESSION_REPUBLISH: Duration = Duration::from_secs(10);

    /// The variables the fake declares, with their units. Packed back to back like
    /// iRacing's, so most of them sit at unaligned offsets.
    const VARS: &[(&str, &str)] = &[
        ("SessionTime", "s"),
        ("SessionTick", ""),
        ("SessionNum", ""),
        ("IsOnTrack", ""),
        ("IsOnTrackCar", ""),
        ("IsInGarage", ""),
        ("IsGarageVisible", ""),
        ("IsReplayPlaying", ""),
        ("PlayerCarIdx", ""),
        ("PlayerTrackSurface", "irsdk_TrkLoc"),
        ("PlayerCarInPitStall", ""),
        ("OnPitRoad", ""),
        ("Lap", ""),
        ("LapCompleted", ""),
        ("LapDist", "m"),
        ("LapDistPct", "%"),
        ("Throttle", "%"),
        ("Brake", "%"),
        ("ThrottleRaw", "%"),
        ("BrakeRaw", "%"),
    ];

    pub fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
        let opts = Options::parse(args)?;
        let lap = Arc::new(sample_lap());
        let mut driver = SimulatedDriver::new(lap, 61);
        let mut sim = Sim::start(&session_yaml(&opts, 0), opts.kerb_event)?;
        println!(
            "fake iRacing: {} at {}{}, 60 Hz. Enter stops cleanly; Ctrl+C leaves the connected flag set, like a crash.",
            opts.car,
            opts.track,
            if opts.config.is_empty() { String::new() } else { format!(" ({})", opts.config) },
        );
        let quit = enter_pressed();
        let start = Instant::now();
        let mut next_republish = SESSION_REPUBLISH;
        let mut tick: i32 = 0;
        loop {
            tick += 1;
            let t = f64::from(tick) / f64::from(TICK_RATE);
            if quit.load(Ordering::Relaxed) || opts.secs.is_some_and(|secs| t > secs) {
                break;
            }
            let row = if t < opts.pit_secs {
                Row::in_pit_stall(tick, t)
            } else {
                let frame = driver.step(1.0 / f64::from(TICK_RATE));
                Row::driving(tick, t, &frame, driver.lap_index())
            };
            sim.publish(&row);
            if start.elapsed() >= next_republish {
                let results = next_republish.as_secs() / SESSION_REPUBLISH.as_secs();
                sim.set_session(&session_yaml(&opts, results));
                next_republish += SESSION_REPUBLISH;
            }
            let due = start + Duration::from_secs_f64(t);
            std::thread::sleep(due.saturating_duration_since(Instant::now()));
        }
        sim.close();
        println!("fake iRacing: stopped cleanly after {:.1} s", start.elapsed().as_secs_f64());
        Ok(())
    }

    #[derive(Debug)]
    struct Options {
        secs: Option<f64>,
        pit_secs: f64,
        car: String,
        track: String,
        /// `TrackName`, iRacing's internal name.
        track_name: String,
        config: String,
        kerb_event: bool,
    }

    impl Options {
        fn parse(mut args: impl Iterator<Item = String>) -> Result<Self, String> {
            let mut o = Self {
                secs: None,
                pit_secs: 0.0,
                car: "Ferrari 296 GT3".into(),
                track: "Silverstone Circuit".into(),
                track_name: "silverstone 2019 gp".into(),
                config: "Arena Grand Prix".into(),
                kerb_event: false,
            };
            while let Some(arg) = args.next() {
                let mut value = || args.next().ok_or_else(|| format!("{arg} needs a value\n{USAGE}"));
                match arg.as_str() {
                    "--secs" => o.secs = Some(seconds(&value()?)?),
                    "--pit" => o.pit_secs = seconds(&value()?)?,
                    "--car" => o.car = value()?,
                    "--track" => {
                        o.track = value()?;
                        o.track_name = o.track.to_lowercase();
                    }
                    "--config" => o.config = value()?,
                    "--kerb-event" => o.kerb_event = true,
                    _ => return Err(format!("unknown option {arg}\n{USAGE}")),
                }
            }
            Ok(o)
        }
    }

    fn seconds(s: &str) -> Result<f64, String> {
        s.parse()
            .ok()
            .filter(|v: &f64| v.is_finite() && *v >= 0.0)
            .ok_or_else(|| format!("not a number of seconds: {s}"))
    }

    /// A flag set when a line is read from stdin. Without a console (EOF) it stays unset.
    fn enter_pressed() -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let set = Arc::clone(&flag);
        std::thread::spawn(move || {
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
                set.store(true, Ordering::Relaxed);
            }
        });
        flag
    }

    /// A telemetry value and its `irsdk_VarType`.
    #[derive(Debug, Clone, Copy)]
    enum Value {
        Bool(bool),
        Int(i32),
        Float(f32),
        Double(f64),
    }

    impl Value {
        fn irsdk_type(self) -> i32 {
            match self {
                Value::Bool(_) => 1,
                Value::Int(_) => 2,
                Value::Float(_) => 4,
                Value::Double(_) => 5,
            }
        }

        fn size(self) -> usize {
            match self {
                Value::Bool(_) => 1,
                Value::Int(_) | Value::Float(_) => 4,
                Value::Double(_) => 8,
            }
        }

        fn write(self, dst: &mut [u8]) {
            match self {
                Value::Bool(v) => dst[0] = u8::from(v),
                Value::Int(v) => dst[..4].copy_from_slice(&v.to_le_bytes()),
                Value::Float(v) => dst[..4].copy_from_slice(&v.to_le_bytes()),
                Value::Double(v) => dst[..8].copy_from_slice(&v.to_le_bytes()),
            }
        }
    }

    /// One telemetry update.
    #[derive(Debug, Clone, Copy, Default)]
    struct Row {
        tick: i32,
        session_time: f64,
        in_pit_stall: bool,
        lap: i32,
        lap_dist_pct: f32,
        throttle: f32,
        brake: f32,
        brake_raw: f32,
    }

    impl Row {
        /// Waiting in the pit stall: iRacing holds the brake (`Brake` 1.0, `BrakeRaw` 0).
        fn in_pit_stall(tick: i32, t: f64) -> Self {
            Self {
                tick,
                session_time: SESSION_TIME_AT_START + t,
                in_pit_stall: true,
                lap: 0,
                lap_dist_pct: PIT_STALL_PCT,
                throttle: 0.0,
                brake: 1.0,
                brake_raw: 0.0,
            }
        }

        fn driving(tick: i32, t: f64, f: &TelemetryFrame, laps_done: u32) -> Self {
            Self {
                tick,
                session_time: SESSION_TIME_AT_START + t,
                in_pit_stall: false,
                lap: i32::try_from(laps_done).unwrap_or(i32::MAX - 1) + 1,
                lap_dist_pct: f.lap_dist_pct as f32,
                throttle: f.throttle,
                brake: f.brake,
                brake_raw: f.brake,
            }
        }

        /// The value of a variable in [`VARS`].
        fn value(&self, name: &str) -> Value {
            match name {
                "SessionTime" => Value::Double(self.session_time),
                "SessionTick" => Value::Int(self.tick),
                "IsOnTrack" | "IsOnTrackCar" => Value::Bool(true),
                "PlayerCarIdx" => Value::Int(PLAYER_CAR_IDX),
                "PlayerTrackSurface" => Value::Int(if self.in_pit_stall { 1 } else { 3 }),
                "PlayerCarInPitStall" | "OnPitRoad" => Value::Bool(self.in_pit_stall),
                "Lap" => Value::Int(self.lap),
                "LapCompleted" => Value::Int((self.lap - 1).max(0)),
                "LapDist" => Value::Float(self.lap_dist_pct * TRACK_LENGTH_M),
                "LapDistPct" => Value::Float(self.lap_dist_pct),
                "Throttle" | "ThrottleRaw" => Value::Float(self.throttle),
                "Brake" => Value::Float(self.brake),
                "BrakeRaw" => Value::Float(self.brake_raw),
                "SessionNum" => Value::Int(0),
                // IsInGarage, IsGarageVisible, IsReplayPlaying.
                _ => Value::Bool(false),
            }
        }
    }

    /// iRacing's shared memory: header, variable headers, session YAML, then three
    /// rotating telemetry buffers.
    struct Sim {
        mem: SharedMemory,
        events: Vec<Event>,
        /// Each variable's offset in a telemetry row.
        offsets: Vec<usize>,
        session_update: i32,
    }

    impl Sim {
        fn start(yaml: &str, kerb_event: bool) -> Result<Self, String> {
            let (mem, existed) = SharedMemory::create(MAP_NAME)?;
            if existed {
                println!("fake iRacing: reusing the mapping a reader still holds open");
            }
            let mut events = vec![Event::create(EVENT_NAME)?];
            if kerb_event {
                events.push(Event::create(KERB_EVENT_NAME)?);
            }
            let mut sim = Self { mem, events, offsets: Vec::new(), session_update: 0 };
            sim.write_headers();
            sim.set_session(yaml);
            put_i32(sim.mem.bytes(), H_STATUS, 1);
            Ok(sim)
        }

        fn write_headers(&mut self) {
            let template = Row::default();
            let mut offset = 0;
            let mem = self.mem.bytes();
            mem.fill(0);
            for (i, (name, unit)) in VARS.iter().enumerate() {
                let value = template.value(name);
                let header = &mut mem[VAR_HEADERS_AT + i * VAR_HEADER_LEN..][..VAR_HEADER_LEN];
                header[0..4].copy_from_slice(&value.irsdk_type().to_le_bytes());
                header[4..8].copy_from_slice(&as_i32(offset).to_le_bytes());
                header[8..12].copy_from_slice(&1i32.to_le_bytes());
                put_str(&mut header[16..48], name);
                put_str(&mut header[112..144], unit);
                self.offsets.push(offset);
                offset += value.size();
            }
            put_i32(mem, H_VER, 2);
            put_i32(mem, H_TICK_RATE, TICK_RATE);
            put_i32(mem, H_NUM_VARS, as_i32(VARS.len()));
            put_i32(mem, H_VAR_HEADER_OFFSET, as_i32(VAR_HEADERS_AT));
            put_i32(mem, H_NUM_BUF, as_i32(NUM_BUF));
            put_i32(mem, H_BUF_LEN, as_i32(offset));
            for i in 0..NUM_BUF {
                put_i32(mem, H_VAR_BUF + i * 16, -1);
                put_i32(mem, H_VAR_BUF + i * 16 + 4, as_i32(BUFFERS_AT + i * BUFFER_STRIDE));
            }
        }

        /// Replaces the session YAML (Latin-1, like iRacing's default) and bumps
        /// `SessionInfoUpdate`.
        fn set_session(&mut self, yaml: &str) {
            let bytes: Vec<u8> = yaml.chars().map(|c| u8::try_from(u32::from(c)).unwrap_or(b'?')).collect();
            let len = bytes.len().min(SESSION_LEN - 1);
            let mem = self.mem.bytes();
            let area = &mut mem[SESSION_AT..SESSION_AT + SESSION_LEN];
            area.fill(0);
            area[..len].copy_from_slice(&bytes[..len]);
            put_i32(mem, H_SESSION_LEN, as_i32(SESSION_LEN));
            put_i32(mem, H_SESSION_OFFSET, as_i32(SESSION_AT));
            self.session_update += 1;
            put_i32(mem, H_SESSION_UPDATE, self.session_update);
        }

        /// Writes the row into the next buffer, then publishes its tick count and signals
        /// the data-valid event.
        fn publish(&mut self, row: &Row) {
            let slot = usize::try_from(row.tick).unwrap_or(0) % NUM_BUF;
            let buffer = BUFFERS_AT + slot * BUFFER_STRIDE;
            let mem = self.mem.bytes();
            for ((name, _), offset) in VARS.iter().zip(&self.offsets) {
                row.value(name).write(&mut mem[buffer + offset..]);
            }
            // Readers take the buffer with the highest tick count, so the row lands first.
            fence(Ordering::Release);
            put_i32(mem, H_VAR_BUF + slot * 16, row.tick);
            for event in &self.events {
                event.set();
            }
        }

        /// Exits the way iRacing does: clears the connected flag before unmapping.
        fn close(mut self) {
            put_i32(self.mem.bytes(), H_STATUS, 0);
        }
    }

    fn put_i32(mem: &mut [u8], at: usize, v: i32) {
        mem[at..at + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Writes a NUL-padded string into a fixed-size field.
    fn put_str(field: &mut [u8], s: &str) {
        let n = s.len().min(field.len() - 1);
        field[..n].copy_from_slice(&s.as_bytes()[..n]);
    }

    fn as_i32(n: usize) -> i32 {
        i32::try_from(n).unwrap_or(i32::MAX)
    }

    /// Session info in iRacing's layout, trimmed to a realistic subset. `results` changes
    /// the lap counts in the results, which is what most of iRacing's updates change.
    fn session_yaml(o: &Options, results: u64) -> String {
        let laps = |n: u64| results + n;
        format!(
            "---
WeekendInfo:
 Encoding: ISO_8859_1
 TrackName: {track_name}
 TrackID: 341
 TrackLength: 5.7961 km
 TrackLengthOfficial: 5.89 km
 TrackDisplayName: {track}
 TrackDisplayShortName: Silverstone
 TrackConfigName: {config}
 TrackCity: Silverstone
 TrackCountry: England
 TrackLatitude: 52.068299 m
 TrackLongitude: -1.023457 m
 TrackNumTurns: 18
 TrackType: road course
 SeriesID: 0
 EventType: Practice
 Category: SportsCar
 WeekendOptions:
  NumStarters: 4
  TimeOfDay: 3:30 pm
  Date: 2026-09-19
 TelemetryOptions:
  TelemetryDiskFile: \"\"

SessionInfo:
 CurrentSessionNum: 0
 Sessions:
 - SessionNum: 0
   SessionLaps: unlimited
   SessionTime: 3600.0000 sec
   SessionType: Practice
   SessionName: PRACTICE
   ResultsPositions:
   - Position: 1
     CarIdx: 1
     Lap: {l1}
     FastestTime: 116.9912
   - Position: 2
     CarIdx: 3
     Lap: {l2}
     FastestTime: 117.2045
   - Position: 3
     CarIdx: 0
     Lap: {l3}
     FastestTime: 118.0480
   ResultsFastestLap:
   - CarIdx: 1
     FastestLap: {l1}
     FastestTime: 116.9912

CameraInfo:
 Groups:
 - GroupNum: 1
   GroupName: Nose
   Cameras:
   - CameraNum: 1
     CameraName: CamNose

DriverInfo:
 DriverCarIdx: {player}
 DriverUserID: 900003
 PaceCarIdx: 5
 DriverCarRedLine: 8000.000
 DriverPitTrkPct: {pit_pct}
 DriverSetupName: baseline.sto
 DriverTires:
 - TireIndex: 0
   TireCompoundType: \"Hard\"
 Drivers:
 - CarIdx: 0
   UserName: Alex Example
   AbbrevName: Example, A
   CarScreenName: BMW M4 GT3 EVO
   CarScreenNameShort: BMW M4 GT3 EVO
   CarClassShortName: GT3 Class
 - CarIdx: 1
   UserName: *Speedy
   AbbrevName: *Speedy
   CarScreenName: Porsche 911 GT3 R (992)
   CarScreenNameShort: Porsche 992 GT3 R
   CarClassShortName: GT3 Class
 - CarIdx: 3
   UserName: Jörg Beispiel
   AbbrevName: Beispiel, J
   CarScreenName: {car}
   CarScreenNameShort: {car}
   CarClassShortName: GT3 Class
 - CarIdx: 5
   UserName: Pace Car
   CarScreenName: Mercedes-AMG GT Safety Car
   CarScreenNameShort: Mercedes-AMG GT SC
   CarIsPaceCar: 1

SplitTimeInfo:
 Sectors:
 - SectorNum: 0
   SectorStartPct: 0.000000

CarSetup:
 UpdateCount: 1
 TiresAero:
  LeftFront:
   StartingPressure: 172.0 kPa

...
",
            track_name = o.track_name,
            track = o.track,
            config = o.config,
            car = o.car,
            player = PLAYER_CAR_IDX,
            pit_pct = PIT_STALL_PCT,
            l1 = laps(3),
            l2 = laps(2),
            l3 = laps(1),
        )
    }

    type Handle = *mut c_void;

    const INVALID_HANDLE_VALUE: Handle = std::ptr::without_provenance_mut(usize::MAX);
    const PAGE_READWRITE: u32 = 0x04;
    const FILE_MAP_ALL_ACCESS: u32 = 0x000F_001F;
    const ERROR_ALREADY_EXISTS: u32 = 183;

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileMappingW(
            file: Handle,
            attributes: *const c_void,
            protect: u32,
            size_high: u32,
            size_low: u32,
            name: *const u16,
        ) -> Handle;
        fn MapViewOfFile(mapping: Handle, access: u32, offset_high: u32, offset_low: u32, bytes: usize) -> *mut c_void;
        fn UnmapViewOfFile(base: *const c_void) -> i32;
        fn CreateEventW(attributes: *const c_void, manual_reset: i32, initial_state: i32, name: *const u16) -> Handle;
        fn SetEvent(event: Handle) -> i32;
        fn CloseHandle(handle: Handle) -> i32;
        fn GetLastError() -> u32;
    }

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    /// A named, page-file backed mapping of [`MAP_SIZE`] bytes.
    struct SharedMemory {
        mapping: Handle,
        view: *mut u8,
    }

    impl SharedMemory {
        /// Creates or opens the mapping; the flag says it already existed.
        fn create(name: &str) -> Result<(Self, bool), String> {
            let name = wide(name);
            let size = u32::try_from(MAP_SIZE).map_err(|e| e.to_string())?;
            // SAFETY: plain Win32 calls with a NUL-terminated name; results are checked.
            unsafe {
                let mapping =
                    CreateFileMappingW(INVALID_HANDLE_VALUE, std::ptr::null(), PAGE_READWRITE, 0, size, name.as_ptr());
                if mapping.is_null() {
                    return Err(format!("CreateFileMappingW failed (error {})", GetLastError()));
                }
                let existed = GetLastError() == ERROR_ALREADY_EXISTS;
                let view = MapViewOfFile(mapping, FILE_MAP_ALL_ACCESS, 0, 0, MAP_SIZE).cast::<u8>();
                if view.is_null() {
                    let error = GetLastError();
                    CloseHandle(mapping);
                    return Err(format!("MapViewOfFile failed (error {error})"));
                }
                Ok((Self { mapping, view }, existed))
            }
        }

        fn bytes(&mut self) -> &mut [u8] {
            // SAFETY: the view is MAP_SIZE bytes, mapped read-write until drop. Readers in
            // other processes only read it.
            unsafe { std::slice::from_raw_parts_mut(self.view, MAP_SIZE) }
        }
    }

    impl Drop for SharedMemory {
        fn drop(&mut self) {
            // SAFETY: both come from `create` and are released once.
            unsafe {
                UnmapViewOfFile(self.view.cast());
                CloseHandle(self.mapping);
            }
        }
    }

    /// A named auto-reset event.
    struct Event(Handle);

    impl Event {
        fn create(name: &str) -> Result<Self, String> {
            let name = wide(name);
            // SAFETY: plain Win32 call with a NUL-terminated name; the result is checked.
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, name.as_ptr()) };
            if handle.is_null() {
                // SAFETY: no preconditions.
                return Err(format!("CreateEventW failed (error {})", unsafe { GetLastError() }));
            }
            Ok(Self(handle))
        }

        fn set(&self) {
            // SAFETY: a valid event handle until drop.
            unsafe { SetEvent(self.0) };
        }
    }

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: from `create`, closed once.
            unsafe { CloseHandle(self.0) };
        }
    }
}
