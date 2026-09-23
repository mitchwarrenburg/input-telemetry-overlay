//! Reference laps: the Garage 61 CSV parser and the lap model.
//!
//! A lap is a list of samples taken at a fixed rate. `LapDistPct` (0..1) is the key
//! that lines a reference sample up with the live car, so laps of any pace compare
//! corner for corner.

use std::fmt;
use std::ops::Range;

/// A brake event starts when the pedal goes above this…
pub const BRAKE_ON: f32 = 0.05;
/// …and ends when it drops below this (hysteresis).
pub const BRAKE_OFF: f32 = 0.02;
/// iRacing's telemetry rate, and the rate Garage 61 exports at.
pub const TELEMETRY_HZ: f64 = 60.0;
/// Without a Gear column: a throttle pulse this short that jumps up from and falls back to
/// [`BLIP_FLOOR`] is taken for an auto-blip. With one, blips are found by the downshifts.
pub const BLIP_MAX_S: f64 = 0.2;
/// Floor for [`BLIP_MAX_S`] pulses.
pub const BLIP_FLOOR: f32 = 0.05;
/// A blip's first sample jumps at least this much; a driver's foot rises far slower
/// (under 0.05 a sample), so short real presses aren't mistaken for blips.
const BLIP_JUMP: f32 = 0.15;
/// While a blip is building, each sample rises at least this much.
const BLIP_RISE: f32 = 0.05;
/// Samples before the gear reads neutral that a blip may already have started.
const BLIP_LEAD: usize = 2;
/// Seconds after the new gear engages that a blip may take to die away (Mustang GT3,
/// Mercedes-AMG GT3: a 0.19→0.70 ramp and a decay over up to 11 samples).
const BLIP_DECAY_S: f64 = 0.2;
/// Columns a reference file must have.
pub const REQUIRED_COLUMNS: [&str; 3] = ["LapDistPct", "Brake", "Throttle"];

/// What the Garage 61 file name tells us about a lap.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LapMeta {
    /// File name without directory or `.csv`.
    pub file_name: String,
    /// `Some("Garage 61")` when the name follows Garage 61's export pattern.
    pub source: Option<String>,
    pub driver: Option<String>,
    pub car: Option<String>,
    pub track: Option<String>,
    /// Seconds.
    pub lap_time: Option<f64>,
}

/// One braking zone: sample range and the highest pedal value inside it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BrakeZone {
    pub start: usize,
    pub end: usize,
    pub peak_idx: usize,
    pub peak: f32,
}

#[derive(Debug, Clone)]
pub struct Lap {
    pub meta: LapMeta,
    /// Samples per second.
    pub hz: f64,
    /// Seconds; `n / hz`.
    pub lap_time: f64,
    /// Metres, from integrating the Speed column (None without one).
    pub track_length_est: Option<f64>,
    /// Lap fraction per sample, non-decreasing.
    pub pct: Vec<f64>,
    pub brake: Vec<f32>,
    pub throttle: Vec<f32>,
    /// m/s, when the file has a Speed column.
    pub speed: Option<Vec<f32>>,
    pub zones: Vec<BrakeZone>,
    /// Latitude/longitude (degrees) where the lap starts, when the file has Lat/Lon.
    /// Compared with the session's track position to confirm the venue.
    pub start_latlon: Option<(f64, f64)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LapError {
    MissingColumns(Vec<&'static str>),
    TooShort,
}

impl fmt::Display for LapError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LapError::MissingColumns(cols) => write!(
                f,
                "Missing column{}: {}. Export the lap from Garage 61 as CSV.",
                if cols.len() > 1 { "s" } else { "" },
                cols.join(", ")
            ),
            LapError::TooShort => {
                write!(f, "That file holds less than 10 seconds of driving. Is it a full lap?")
            }
        }
    }
}

impl std::error::Error for LapError {}

/// Parses Garage 61 export names:
/// `Garage 61 - Driver - Car - Track (Layout) - 01.55.992 - ULID.csv`, the older
/// `Garage_61__Driver__Car__Track__01.55.992__ULID.csv`, and either with a browser's
/// `(1)` re-download suffix. Names that don't follow the pattern keep only `file_name`.
pub fn parse_file_name(file_name: &str) -> LapMeta {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    let base = strip_download_suffix(strip_csv_ext(base));
    let mut meta = LapMeta { file_name: base.to_string(), ..Default::default() };
    let parts: Vec<String> = if base.starts_with("Garage 61 - ") {
        base.split(" - ").map(|p| p.trim().to_string()).collect()
    } else if base.starts_with("Garage_61__") {
        base.split("__").map(|p| p.replace('_', " ").trim().to_string()).collect()
    } else {
        return meta;
    };
    // Driver, car, one or more track parts (track names can contain " - "), lap time.
    // The car comes first: car names end in ")" as often as tracks do, e.g. "(992)".
    if let Some(ti) = parts.iter().position(|p| parse_g61_time(p).is_some()).filter(|&ti| ti >= 4) {
        meta.source = Some("Garage 61".into());
        meta.driver = Some(parts[1].clone());
        meta.car = Some(parts[2].clone());
        meta.track = Some(parts[3..ti].join(" - "));
        meta.lap_time = parse_g61_time(&parts[ti]);
    }
    meta
}

fn strip_csv_ext(s: &str) -> &str {
    if s.len() >= 4 && s[s.len() - 4..].eq_ignore_ascii_case(".csv") { &s[..s.len() - 4] } else { s }
}

/// `name(1)` → `name` (browsers add this, with no space, when a file is downloaded again).
fn strip_download_suffix(s: &str) -> &str {
    if let Some(open) = s.strip_suffix(')').and_then(|t| t.rfind('(')) {
        let inner = &s[open + 1..s.len() - 1];
        if !inner.is_empty() && inner.chars().all(|c| c.is_ascii_digit()) && !s[..open].ends_with(' ') {
            return &s[..open];
        }
    }
    s
}

/// `01.55.992` → 115.992 s.
fn parse_g61_time(s: &str) -> Option<f64> {
    let mut it = s.trim().split('.');
    let (m, sec, ms) = (it.next()?, it.next()?, it.next()?);
    if it.next().is_some()
        || !(1..=2).contains(&m.len())
        || sec.len() != 2
        || ms.len() != 3
        || !(m.chars().chain(sec.chars()).chain(ms.chars())).all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some(m.parse::<f64>().ok()? * 60.0 + sec.parse::<f64>().ok()? + ms.parse::<f64>().ok()? / 1000.0)
}

/// `115.992` → `1:55.992`.
pub fn format_lap_time(sec: f64) -> String {
    if !sec.is_finite() || sec < 0.0 {
        return "–".into();
    }
    let ms_total = (sec * 1000.0).round() as u64;
    let (m, rem) = (ms_total / 60_000, ms_total % 60_000);
    format!("{}:{:02}.{:03}", m, rem / 1000, rem % 1000)
}

/// Flattens the auto-blips in a throttle trace. Garage 61 exports iRacing's `Throttle`,
/// where the car opens the throttle itself on every downshift while the gearbox reads
/// neutral; the live overlay reads the pedal (`ThrottleRaw`), which doesn't show them.
/// With `gear` (the export's Gear column) each downshift's blip is found directly; without
/// it, short pulses that jump off the floor are taken instead (see [`BLIP_MAX_S`]). Each
/// blip becomes a straight line between the driver's throttle either side. Returns how
/// many were removed.
pub fn remove_blips(throttle: &mut [f32], gear: Option<&[i32]>, hz: f64) -> usize {
    match gear {
        Some(gear) if gear.len() == throttle.len() && gear.contains(&0) => remove_downshift_blips(throttle, gear, hz),
        _ => remove_short_pulses(throttle, hz),
    }
}

/// Blips around each neutral stretch between a higher and a lower gear.
fn remove_downshift_blips(throttle: &mut [f32], gear: &[i32], hz: f64) -> usize {
    let n = throttle.len();
    let decay = (BLIP_DECAY_S * hz).round() as usize;
    let mut removed = 0;
    let mut i = 1;
    while i < n {
        if gear[i] != 0 || gear[i - 1] <= 0 {
            i += 1;
            continue;
        }
        let neutral = i;
        while i < n && gear[i] == 0 {
            i += 1;
        }
        let engaged = i;
        if engaged == n || gear[engaged] <= 0 || gear[engaged] >= gear[neutral - 1] {
            continue; // an upshift, or the lap ends mid-shift
        }
        // It may start a sample or two before the gear reads neutral…
        let mut start = neutral;
        while start > 1 && neutral - start < BLIP_LEAD && throttle[start - 1] > throttle[start - 2] + BLIP_RISE {
            start -= 1;
        }
        // …and dies away after the new gear engages: it ends where the throttle stops falling.
        let mut end = engaged;
        while end + 1 < n && end < engaged + decay && throttle[end + 1] < throttle[end] - 0.01 {
            end += 1;
        }
        let before = throttle[start - 1];
        if throttle[start..end].iter().all(|&v| v <= before + 0.02) {
            continue; // nothing rose: no blip on this shift
        }
        bridge(throttle, start - 1, end);
        removed += 1;
    }
    removed
}

/// Without gears: pulses from the floor that jump at once and are gone within [`BLIP_MAX_S`].
fn remove_short_pulses(throttle: &mut [f32], hz: f64) -> usize {
    let longest = (BLIP_MAX_S * hz).round().max(1.0) as usize;
    let mut removed = 0;
    let mut i = 1;
    while i < throttle.len() {
        if throttle[i] > BLIP_FLOOR && throttle[i - 1] <= BLIP_FLOOR {
            let Some(end) = (i..throttle.len()).find(|&j| throttle[j] <= BLIP_FLOOR) else { break };
            if end - i <= longest && throttle[i] - throttle[i - 1] >= BLIP_JUMP {
                bridge(throttle, i - 1, end);
                removed += 1;
            }
            i = end;
        }
        i += 1;
    }
    removed
}

/// A straight line from sample `from` to sample `to`, replacing those between.
fn bridge(values: &mut [f32], from: usize, to: usize) {
    let (a, b) = (values[from], values[to]);
    let span = (to - from) as f32;
    for (k, v) in values[from + 1..to].iter_mut().enumerate() {
        *v = a + (b - a) * (k + 1) as f32 / span;
    }
}

/// Brake zones with hysteresis ([`BRAKE_ON`] / [`BRAKE_OFF`]).
pub fn find_zones(brake: &[f32]) -> Vec<BrakeZone> {
    let mut zones = Vec::new();
    let mut z: Option<BrakeZone> = None;
    for (i, &b) in brake.iter().enumerate() {
        match z.as_mut() {
            None => {
                if b > BRAKE_ON {
                    z = Some(BrakeZone { start: i, end: i, peak_idx: i, peak: b });
                }
            }
            Some(cur) => {
                if b > cur.peak {
                    cur.peak = b;
                    cur.peak_idx = i;
                }
                if b < BRAKE_OFF {
                    cur.end = i;
                    zones.push(*cur);
                    z = None;
                }
            }
        }
    }
    if let Some(mut cur) = z {
        cur.end = brake.len().saturating_sub(1);
        zones.push(cur);
    }
    zones
}

/// Parses a Garage 61 lap export (any CSV with LapDistPct, Brake and Throttle columns).
pub fn parse_garage61_csv(text: &str, file_name: &str) -> Result<Lap, LapError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    let header: Vec<&str> = lines.next().unwrap_or("").split(',').map(str::trim).collect();
    let col = |name: &str| header.iter().position(|h| h.eq_ignore_ascii_case(name));
    let missing: Vec<&'static str> = REQUIRED_COLUMNS.iter().copied().filter(|c| col(c).is_none()).collect();
    if !missing.is_empty() {
        return Err(LapError::MissingColumns(missing));
    }
    let (i_pct, i_brake, i_thr) = (col("LapDistPct").unwrap(), col("Brake").unwrap(), col("Throttle").unwrap());
    let i_spd = col("Speed");
    let (i_lat, i_lon) = (col("Lat"), col("Lon"));
    let i_gear = col("Gear");

    let (mut pct, mut brake, mut throttle, mut speed) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let mut latlon: Vec<Option<(f64, f64)>> = Vec::new();
    let mut gear: Vec<i32> = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        let f: Vec<&str> = line.split(',').collect();
        let num = |i: usize| f.get(i).and_then(|v| v.trim().parse::<f64>().ok()).filter(|v| v.is_finite());
        let (Some(p), Some(b), Some(t)) = (num(i_pct), num(i_brake), num(i_thr)) else { continue };
        pct.push(p);
        brake.push(b.clamp(0.0, 1.0) as f32);
        throttle.push(t.clamp(0.0, 1.0) as f32);
        speed.push(i_spd.and_then(num).unwrap_or(0.0) as f32);
        latlon.push(i_lat.and_then(num).zip(i_lon.and_then(num)).filter(|&(la, lo)| la != 0.0 || lo != 0.0));
        gear.push(i_gear.and_then(num).map_or(-1, |g| g as i32));
    }

    // Exports often carry a sample or two from the neighbouring lap: keep the longest run
    // between start/finish wraps (the later one on a tie). One pass, so a file full of
    // wraps can't stall the parse.
    let mut keep = 0..0;
    let mut start = 0;
    for i in 1..=pct.len() {
        if i == pct.len() || pct[i] < pct[i - 1] - 0.5 {
            if i - start >= keep.len() {
                keep = start..i;
            }
            start = i;
        }
    }
    keep_range(&mut pct, &keep);
    keep_range(&mut brake, &keep);
    keep_range(&mut throttle, &keep);
    keep_range(&mut speed, &keep);
    keep_range(&mut latlon, &keep);
    keep_range(&mut gear, &keep);
    if (pct.len() as f64) < TELEMETRY_HZ * 10.0 {
        return Err(LapError::TooShort);
    }
    for i in 1..pct.len() {
        if pct[i] < pct[i - 1] {
            pct[i] = pct[i - 1]; // float jitter
        }
    }

    let meta = parse_file_name(file_name);
    let n = pct.len();
    // No time column in the export: derive the rate from the lap time in the file name.
    let mut hz = meta.lap_time.map_or(TELEMETRY_HZ, |lt| n as f64 / lt);
    if !(20.0..=400.0).contains(&hz) {
        hz = TELEMETRY_HZ;
    }
    let lap_time = meta.lap_time.unwrap_or(n as f64 / hz);
    let track_length_est = i_spd.map(|_| speed.iter().map(|&v| v as f64 / hz).sum::<f64>());
    remove_blips(&mut throttle, i_gear.map(|_| gear.as_slice()), hz);
    let start_latlon = pct.iter().zip(&latlon).take_while(|(p, _)| **p < 0.01).find_map(|(_, ll)| *ll);

    Ok(Lap {
        zones: find_zones(&brake),
        meta,
        hz,
        lap_time,
        track_length_est,
        pct,
        brake,
        throttle,
        speed: i_spd.map(|_| speed),
        start_latlon,
    })
}

/// Keeps `range` of `v`.
fn keep_range<T>(v: &mut Vec<T>, range: &Range<usize>) {
    v.truncate(range.end);
    v.drain(..range.start);
}

impl Lap {
    pub fn n(&self) -> usize {
        self.pct.len()
    }

    /// Fractional sample index at a lap fraction. Values before the first sample
    /// resolve against the previous lap's tail (negative index) and values after the
    /// last against the next lap's head, so the result is continuous across S/F.
    pub fn index_at_pct(&self, p: f64) -> f64 {
        let a = &self.pct;
        let n = a.len();
        if p < a[0] {
            let prev = a[n - 1] - 1.0;
            return -1.0 + (p - prev) / (a[0] - prev).max(1e-9);
        }
        if p >= a[n - 1] {
            let next = a[0] + 1.0;
            return (n - 1) as f64 + (p - a[n - 1]) / (next - a[n - 1]).max(1e-9);
        }
        // Last index with a[i] <= p.
        let lo = a.partition_point(|&v| v <= p) - 1;
        let hi = lo + 1;
        lo as f64 + (p - a[lo]) / (a[hi] - a[lo]).max(1e-9)
    }

    /// First sample index whose lap fraction is `>= p`.
    pub fn lower_bound(&self, p: f64) -> usize {
        self.pct.partition_point(|&v| v < p)
    }

    /// Seconds into the lap at a lap fraction (may be slightly negative just after S/F).
    pub fn time_at_pct(&self, p: f64) -> f64 {
        self.index_at_pct(p) / self.hz
    }

    /// Lap time as `m:ss.mmm`.
    pub fn lap_time_text(&self) -> String {
        format_lap_time(self.lap_time)
    }

    /// Four-column CSV copy of the lap (what the library stores for non-Garage 61 sources).
    pub fn to_csv(&self) -> String {
        let mut out = String::from("LapDistPct,Brake,Throttle,Speed\n");
        for i in 0..self.n() {
            let spd = self.speed.as_ref().map_or(0.0, |s| s[i]);
            out.push_str(&format!("{:.7},{:.4},{:.4},{:.2}\n", self.pct[i], self.brake[i], self.throttle[i], spd));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = include_str!("../assets/sample-laps/silverstone-gp-ferrari-296-gt3.csv");
    const G61_NAME: &str =
        "Garage 61 - Sample Lap - Ferrari 296 GT3 - Silverstone Circuit (Grand Prix) - 01.55.992 - SAMPLE.csv";

    #[test]
    fn parses_garage61_file_name() {
        let m = parse_file_name(&format!("C:\\laps\\{G61_NAME}"));
        assert_eq!(m.source.as_deref(), Some("Garage 61"));
        assert_eq!(m.driver.as_deref(), Some("Sample Lap"));
        assert_eq!(m.car.as_deref(), Some("Ferrari 296 GT3"));
        assert_eq!(m.track.as_deref(), Some("Silverstone Circuit (Grand Prix)"));
        assert!((m.lap_time.unwrap() - 115.992).abs() < 1e-9);
        assert!(m.file_name.ends_with("SAMPLE"));
    }

    #[test]
    fn track_names_with_dashes_survive() {
        let m = parse_file_name("Garage 61 - A B - Car X - Road - America - Full - 02.01.500 - ID.CSV");
        assert_eq!(m.track.as_deref(), Some("Road - America - Full"));
        assert!((m.lap_time.unwrap() - 121.5).abs() < 1e-9);
    }

    #[test]
    fn older_and_redownloaded_names_parse() {
        let m = parse_file_name(
            "Garage_61__A_Driver__Ford_Mustang_GT4__Summit_Point_Raceway__01.27.017__01K5AAAAAAAAAAAAAAAAAAAAAA.csv",
        );
        assert_eq!(m.driver.as_deref(), Some("A Driver"));
        assert_eq!(m.car.as_deref(), Some("Ford Mustang GT4"));
        assert_eq!(m.track.as_deref(), Some("Summit Point Raceway"));
        let m = parse_file_name(&G61_NAME.replace(".csv", "(1).csv"));
        assert_eq!(m.track.as_deref(), Some("Silverstone Circuit (Grand Prix)"));
        assert!(m.file_name.ends_with("SAMPLE"));
        // A car name ending in ")" stays the car.
        let m = parse_file_name("Garage 61 - D - Porsche 911 GT3 R (992) - Lime Rock Park - 01.00.000 - X.csv");
        assert_eq!((m.car.as_deref(), m.track.as_deref()), (Some("Porsche 911 GT3 R (992)"), Some("Lime Rock Park")));
    }

    #[test]
    fn other_file_names_keep_only_the_name() {
        let m = parse_file_name("my lap.csv");
        assert_eq!(m, LapMeta { file_name: "my lap".into(), ..Default::default() });
    }

    #[test]
    fn formats_lap_times() {
        assert_eq!(format_lap_time(115.992), "1:55.992");
        assert_eq!(format_lap_time(59.9996), "1:00.000");
        assert_eq!(format_lap_time(f64::NAN), "–");
    }

    #[test]
    fn parses_the_sample_lap() {
        let lap = parse_garage61_csv(SAMPLE, G61_NAME).unwrap();
        // The export ends with one sample from the next lap; it must be dropped.
        assert_eq!(lap.n(), 6959);
        assert!((lap.hz - 59.996).abs() < 0.01);
        assert!((lap.lap_time - 115.992).abs() < 1e-9);
        let len = lap.track_length_est.unwrap();
        assert!((5700.0..5900.0).contains(&len), "{len}");
        assert!(lap.pct.windows(2).all(|w| w[0] <= w[1]));
        assert!(lap.brake.iter().chain(&lap.throttle).all(|v| (0.0..=1.0).contains(v)));
        let peaks: Vec<u32> = lap.zones.iter().map(|z| (z.peak * 100.0).round() as u32).collect();
        assert_eq!(peaks, [68, 46, 72, 48, 44, 9, 59, 33, 67, 77]);
        let (lat, lon) = lap.start_latlon.unwrap();
        assert!((lat - 52.0684).abs() < 0.001 && (lon + 1.0235).abs() < 0.001, "{lat},{lon}");
    }

    #[test]
    fn index_at_pct_is_continuous_across_start_finish() {
        let lap = parse_garage61_csv(SAMPLE, G61_NAME).unwrap();
        let n = lap.n() as f64;
        assert!(lap.index_at_pct(0.0) < 0.0 && lap.index_at_pct(0.0) > -1.0);
        assert!(lap.index_at_pct(0.99999) > n - 1.0 && lap.index_at_pct(0.99999) < n);
        let mid = lap.index_at_pct(0.5);
        assert!((mid - lap.lower_bound(0.5) as f64).abs() <= 1.0);
        // Monotonic over a sweep.
        let mut prev = f64::NEG_INFINITY;
        for k in 0..=1000 {
            let v = lap.index_at_pct(k as f64 / 1000.0 * 0.99999);
            assert!(v >= prev);
            prev = v;
        }
    }

    #[test]
    fn missing_columns_are_reported() {
        let err = parse_garage61_csv("Speed,Foo\n1,2\n", "x.csv").unwrap_err();
        assert_eq!(err, LapError::MissingColumns(vec!["LapDistPct", "Brake", "Throttle"]));
        assert!(err.to_string().starts_with("Missing columns: LapDistPct, Brake, Throttle."));
    }

    #[test]
    fn short_files_are_rejected() {
        let mut csv = String::from("LapDistPct,Brake,Throttle\n");
        for i in 0..100 {
            csv.push_str(&format!("{},0,1\n", i as f64 / 100.0));
        }
        assert_eq!(parse_garage61_csv(&csv, "x.csv").unwrap_err(), LapError::TooShort);
    }

    #[test]
    fn leading_samples_from_previous_lap_are_dropped_and_rate_defaults_to_60() {
        let mut csv = String::from("\u{feff}lapdistpct,BRAKE,Throttle\n0.9999,0,1\n");
        for i in 0..1200 {
            csv.push_str(&format!("{:.6},0,1\n", i as f64 / 1200.0));
        }
        let lap = parse_garage61_csv(&csv, "no-name.csv").unwrap();
        assert_eq!(lap.n(), 1200);
        assert_eq!(lap.hz, TELEMETRY_HZ);
        assert!((lap.lap_time - 20.0).abs() < 1e-9);
        assert!(lap.track_length_est.is_none());
    }

    #[test]
    fn many_wraps_parse_in_linear_time() {
        // 80,000 rows flipping between 0.9 and 0.1 (took 14 s to parse when each wrap
        // re-copied the rest of the file), then the same flicker before a real lap.
        let mut flicker = String::from("LapDistPct,Brake,Throttle\n");
        for i in 0..80_000 {
            flicker.push_str(if i % 2 == 0 { "0.1,0,1\n" } else { "0.9,0,1\n" });
        }
        let start = std::time::Instant::now();
        assert_eq!(parse_garage61_csv(&flicker, "x.csv").unwrap_err(), LapError::TooShort);
        let mut csv = flicker.clone();
        for i in 0..7000 {
            csv.push_str(&format!("{:.6},0.1,0.9\n", i as f64 / 7000.0));
        }
        let lap = parse_garage61_csv(&csv, "x.csv").unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "{:?}", start.elapsed());
        assert_eq!(lap.n(), 7000);
        assert_eq!((lap.pct[0], lap.brake[0]), (0.0, 0.1));
    }

    #[test]
    fn the_longest_run_between_wraps_is_kept() {
        let mut csv = String::from("LapDistPct,Brake,Throttle\n");
        let mut rows = |from: f64, n: usize, step: f64, brake: f64| {
            for i in 0..n {
                csv.push_str(&format!("{:.6},{brake},0\n", from + i as f64 * step));
            }
        };
        rows(0.95, 300, 0.0001, 0.1); // the end of the previous lap
        rows(0.0, 700, 1.0 / 700.0, 0.2); // the lap
        rows(0.0, 699, 1.0 / 700.0, 0.3); // the next lap, one sample short
        let lap = parse_garage61_csv(&csv, "x.csv").unwrap();
        assert_eq!(lap.n(), 700);
        assert!(lap.brake.iter().all(|&b| b == 0.2));
    }

    #[test]
    fn csv_round_trip_keeps_samples() {
        let lap = parse_garage61_csv(SAMPLE, G61_NAME).unwrap();
        let again = parse_garage61_csv(&lap.to_csv(), G61_NAME).unwrap();
        assert_eq!(again.n(), lap.n());
        assert_eq!(again.zones.len(), lap.zones.len());
    }

    /// The sample lap's raw Throttle and Gear columns.
    fn raw_throttle_and_gear() -> (Vec<f32>, Vec<i32>) {
        let mut lines = SAMPLE.lines();
        let header: Vec<&str> = lines.next().unwrap().split(',').collect();
        let (it, ig) =
            (header.iter().position(|&h| h == "Throttle").unwrap(), header.iter().position(|&h| h == "Gear").unwrap());
        let rows: Vec<Vec<&str>> = lines.map(|l| l.split(',').collect()).collect();
        let n = rows.len() - 1; // the last row is the next lap's
        let t = rows[..n].iter().map(|r| r[it].parse::<f32>().unwrap().clamp(0.0, 1.0)).collect();
        let g = rows[..n].iter().map(|r| r[ig].parse().unwrap()).collect();
        (t, g)
    }

    #[test]
    fn downshift_blips_are_flattened_and_nothing_else() {
        let lap = parse_garage61_csv(SAMPLE, G61_NAME).unwrap();
        let (raw, gear) = raw_throttle_and_gear();
        let under_braking = lap.throttle.iter().zip(&lap.brake).filter(|&(&t, &b)| b > 0.4 && t > 0.3);
        assert_eq!(under_braking.count(), 0, "every blip under braking is gone");
        // Only samples next to a shift through neutral change.
        for (i, (&t, &r)) in lap.throttle.iter().zip(&raw).enumerate() {
            if (t - r).abs() > 1e-6 {
                let near = gear[i.saturating_sub(3)..(i + 15).min(gear.len())].contains(&0);
                assert!(near, "sample {i} changed ({r} -> {t}) away from any shift");
            }
        }
        // Row 3664: 5th to 4th off the throttle, a 52-54% blip for two samples, now flat.
        let i = (3600..3700).find(|&i| raw[i] > 0.5 && gear[i] == 0).unwrap();
        assert_eq!(lap.throttle[i..i + 2], [0.0, 0.0]);
    }

    #[test]
    fn ramped_blips_from_other_cars_are_flattened() {
        // Mustang GT3 / Mercedes-AMG GT3: the blip ramps up a sample before neutral shows
        // and decays for three after the new gear engages.
        let mut t = vec![0.0; 6];
        t.extend([0.194, 0.428, 0.661, 0.70, 0.70, 0.583, 0.35, 0.117, 0.0, 0.0, 0.0]);
        let mut g = vec![2; 7];
        g.extend([0, 0, 0, 0, 1, 1, 1, 1, 1, 1]);
        assert_eq!(remove_blips(&mut t, Some(&g), 60.0), 1);
        assert!(t.iter().all(|&v| v < 0.01), "{t:?}");

        // A blip on trail throttle runs between the driver's 23% before and 17% after.
        let mut t = vec![0.25, 0.24, 0.227, 0.521, 0.417, 0.17, 0.17];
        let g = [5, 5, 5, 0, 0, 4, 4];
        assert_eq!(remove_blips(&mut t, Some(&g), 60.0), 1);
        assert!(t[3] < 0.227 && t[3] > 0.17 && t[4] < t[3] && t[4] > 0.17, "{t:?}");

        // Upshifts at full throttle are left alone.
        let mut t = vec![1.0; 10];
        let g = [3, 3, 3, 0, 0, 4, 4, 4, 4, 4];
        assert_eq!(remove_blips(&mut t, Some(&g), 60.0), 0);
        assert!(t.iter().all(|&v| v == 1.0));
    }

    #[test]
    fn without_gears_only_sudden_short_pulses_go() {
        let mut t = vec![0.0, 0.0, 0.52, 0.65, 0.0, 0.0, 0.03, 0.75, 0.04, 0.0];
        // A driver's short, slow press: 0.05 a sample up to 0.3 and back, 0.2 s.
        t.extend([0.05, 0.1, 0.15, 0.2, 0.25, 0.3, 0.25, 0.2, 0.15, 0.1, 0.05, 0.0]);
        let before = t.clone();
        assert_eq!(remove_blips(&mut t, None, 60.0), 2);
        assert!(t[2] <= BLIP_FLOOR && t[3] <= BLIP_FLOOR, "{t:?}");
        assert!((t[7] - 0.035).abs() < 1e-6, "between its neighbours: {}", t[7]);
        assert_eq!(t[10..], before[10..], "the slow press stays");
        let mut edges = vec![1.0, 1.0, 0.0, 0.0, 0.9];
        assert_eq!(remove_blips(&mut edges, None, 60.0), 0, "a trace ending on the throttle");
    }
}
