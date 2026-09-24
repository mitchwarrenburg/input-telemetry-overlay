//! The brake point countdown: counts 3-2-1-BRAKE into each of the reference lap's brake
//! points, shows that zone's target peak, and grades when your brake went on. The model
//! of `prototype/src/brake-cue.js`.
//!
//! Everything runs on the reference lap's clock (sample index ÷ Hz at your lap position),
//! which needs only the lap fraction from the sim. If you're slower down the straight the
//! counts stretch, but BRAKE always lands on the reference brake point.

use std::collections::{HashMap, VecDeque};

use crate::lap::Lap;
use crate::trace::{BrakeEvent, LiveTrace};

/// "Very" early or late: past this many times the good window.
const VERY: f64 = 3.0;
/// Seconds the bar shows your grade's colour after you brake…
pub const FLASH_HOLD: f64 = 0.8;
/// …then fades out over this.
pub const FLASH_DRAIN: f64 = 0.3;
/// Seconds the compact bar shows your final pressure after you release.
pub const FINAL_HOLD: f64 = 3.0;
/// A brake-on this many seconds before a brake point still counts as that zone's.
const MATCH_BEFORE: f64 = 4.0;
/// Results kept (about eight laps of a twelve-zone track).
const MAX_RESULTS: usize = 120;
/// Brake-ons handed to the graph to underline.
const MARKS: usize = 24;
/// The car moving back on the reference clock by more than this is a new trace (a tow,
/// a reset, a new session): what was graded before doesn't apply.
const BACKWARDS_S: f64 = 1.0;

/// How your brake-on compared with the reference's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Grade {
    VeryEarly,
    Early,
    Good,
    /// Within the (tighter) perfect window.
    Perfect,
    Late,
    VeryLate,
    /// You didn't brake where the reference did.
    NoBrake,
}

impl Grade {
    /// `dt`: your brake-on minus the reference's, reference seconds (`None`: no brake-on).
    /// `tol`: the ± good window; `perfect`: the ± perfect window inside it.
    pub fn of(dt: Option<f64>, tol: f64, perfect: f64) -> Self {
        let Some(dt) = dt else { return Grade::NoBrake };
        let a = dt.abs();
        if a <= perfect {
            Grade::Perfect
        } else if a <= tol {
            Grade::Good
        } else if a <= VERY * tol {
            if dt < 0.0 { Grade::Early } else { Grade::Late }
        } else if dt < 0.0 {
            Grade::VeryEarly
        } else {
            Grade::VeryLate
        }
    }

    /// The chip's text. Chevrons point the way the graph does (early = behind the car =
    /// left), so the grade reads without colour.
    pub fn chip(self) -> &'static str {
        match self {
            Grade::VeryEarly => "«« EARLY",
            Grade::Early => "« EARLY",
            Grade::Good => "GOOD",
            Grade::Perfect => "PERFECT",
            Grade::Late => "LATE »",
            Grade::VeryLate => "LATE »»",
            Grade::NoBrake => "NO BRAKE",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Grade::VeryEarly => "Very early",
            Grade::Early => "Early",
            Grade::Good => "Good",
            Grade::Perfect => "Perfect",
            Grade::Late => "Late",
            Grade::VeryLate => "Very late",
            Grade::NoBrake => "No brake",
        }
    }
}

/// The countdown's settings, in the model's units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CueConfig {
    /// Seconds for the three counts.
    pub lead: f64,
    /// Show BRAKE this many seconds before the reference brake point (reaction allowance).
    pub early: f64,
    /// Zones whose reference peak is below this (0..1) get no countdown.
    pub min_peak: f32,
    /// ± seconds of the good window; very early/late past 3×.
    pub tol: f64,
    /// ± seconds of the perfect window, inside the good one.
    pub perfect: f64,
}

impl Default for CueConfig {
    fn default() -> Self {
        Self { lead: 3.0, early: 0.0, min_peak: 0.15, tol: 0.08, perfect: 0.03 }
    }
}

/// What the window shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CueMode {
    NoReference,
    /// The reference has no zone above the minimum peak.
    NoZones,
    /// No car to count down for (not on track, or no telemetry yet).
    NoCar,
    /// Between zones: the distance to the next brake point.
    Idle,
    /// Counting 3, 2, 1.
    Countdown,
    /// At or past the brake point, not braking yet.
    Brake,
    /// On the brake in the current zone (or the grade still showing after).
    Braking,
}

/// The bar in your grade's colour just after you brake: `alpha` is 1 for
/// [`FLASH_HOLD`], then fades to 0 over [`FLASH_DRAIN`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Flash {
    pub grade: Grade,
    pub alpha: f32,
}

/// Your final pressure in the zone you just left, for [`FINAL_HOLD`] seconds after you
/// release (compact mode's readout).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FinalPeak {
    pub peak: f32,
    pub target: f32,
    pub age: f64,
}

/// The timing row: the zone being decided, else the last one graded.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Verdict {
    /// 1-based, in lap order among the counted zones.
    pub zone_no: usize,
    pub grade: Grade,
    /// Your brake-on minus the reference's: reference seconds and metres.
    pub dt: Option<f64>,
    pub dm: Option<f64>,
    /// Past the brake point and not braking yet: `dt` is counting up.
    pub pending: bool,
    /// It's the zone the window is on.
    pub current: bool,
    /// Your peak so far in that zone.
    pub peak: Option<f32>,
    /// That zone's reference peak.
    pub target: f32,
}

/// One per counted zone in the header strip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Pip {
    /// This lap's grade there, else last lap's.
    pub grade: Option<Grade>,
    /// From last lap.
    pub stale: bool,
    /// The zone the window is on.
    pub current: bool,
}

/// A graded brake-on, for the graph to underline from the reference brake point to yours.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mark {
    /// The reference lap copy (laps since the trace started) and zone index.
    pub lap: i64,
    pub zone: usize,
    pub grade: Grade,
    /// Where your brake went on: session seconds and cumulative metres.
    pub on_t: f64,
    pub on_d: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CueState {
    pub mode: CueMode,
    /// 3, 2 or 1 while counting; 0 otherwise.
    pub beat: u8,
    /// How much of the bar is filled, 0..1.
    pub fill: f32,
    /// How much of it a short straight skipped (hatched), 0..1.
    pub join: f32,
    pub zone_no: usize,
    pub zone_count: usize,
    /// Seconds to the moment BRAKE shows (negative once it has).
    pub until_brake: f64,
    /// Metres to the zone's brake point.
    pub dist: f64,
    /// The zone's reference peak, 0..1.
    pub target: Option<f32>,
    /// Your brake pedal now, 0..1.
    pub live: f32,
    /// Your peak in the zone while braking.
    pub peak: Option<f32>,
    pub flash: Option<Flash>,
    pub final_peak: Option<FinalPeak>,
    pub verdict: Option<Verdict>,
    pub pips: Vec<Pip>,
    /// Indices of the zones that get a countdown, in lap order.
    pub cue_zones: Vec<usize>,
    pub marks: Vec<Mark>,
}

impl CueState {
    fn empty(mode: CueMode) -> Self {
        Self {
            mode,
            beat: 0,
            fill: 0.0,
            join: 0.0,
            zone_no: 0,
            zone_count: 0,
            until_brake: f64::INFINITY,
            dist: 0.0,
            target: None,
            live: 0.0,
            peak: None,
            flash: None,
            final_peak: None,
            verdict: None,
            pips: Vec::new(),
            cue_zones: Vec::new(),
            marks: Vec::new(),
        }
    }
}

/// Where the car is.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CuePosition {
    /// Session seconds.
    pub t: f64,
    /// Cumulative lap position (laps since the trace started + lap fraction).
    pub lap_pos: f64,
    /// Brake pedal, 0..1.
    pub brake: f32,
}

/// Zone `k` on lap copy `m`, with its brake-on (`s`) and brake-off (`e`) on the
/// reference clock.
#[derive(Debug, Clone, Copy)]
struct Occurrence {
    m: i64,
    k: usize,
    s: f64,
    e: f64,
}

impl Occurrence {
    fn key(&self) -> (i64, usize) {
        (self.m, self.k)
    }
}

#[derive(Debug, Clone)]
struct Graded {
    m: i64,
    k: usize,
    /// `None`: driven through without braking.
    dt: Option<f64>,
    dm: Option<f64>,
    /// The brake event, refreshed from the trace while it's there.
    event: Option<BrakeEvent>,
}

/// Grades brake-ons and works out what the window shows. Keep one per reference lap and
/// live trace; it resets itself when either changes.
#[derive(Debug, Default)]
pub struct BrakeCue {
    results: HashMap<(i64, usize), Graded>,
    /// Result keys, oldest first.
    order: VecDeque<(i64, usize)>,
    /// Brake-ons up to this session time have been looked at.
    seen_to: f64,
    /// Reference clock when grading started: zones before it aren't "no brake".
    start: Option<f64>,
    last_clock: f64,
    /// The lap and track length the results are for.
    key: Option<(usize, u64, u64)>,
}

impl BrakeCue {
    pub fn new() -> Self {
        Self { seen_to: f64::NEG_INFINITY, ..Self::default() }
    }

    /// Forgets every grade (a new trace or reference).
    pub fn reset(&mut self) {
        *self = Self::new();
    }

    /// Updates the grades with the trace's new brake-ons and returns what the window
    /// shows. `lap`: the reference (`None` without one); `track_length`: metres per lap,
    /// as the trace's distances were computed.
    pub fn update(
        &mut self,
        now: Option<CuePosition>,
        live: &LiveTrace,
        lap: Option<&Lap>,
        track_length: f64,
        cfg: &CueConfig,
    ) -> CueState {
        let Some(lap) = lap.filter(|l| l.n() >= 2 && l.hz > 0.0 && l.lap_time > 0.0) else {
            return CueState::empty(CueMode::NoReference);
        };
        let key = (std::ptr::from_ref(lap) as usize, lap.lap_time.to_bits(), track_length.to_bits());
        if self.key != Some(key) {
            self.reset();
            self.key = Some(key);
        }
        let cue_zones: Vec<usize> = (0..lap.zones.len()).filter(|&k| lap.zones[k].peak >= cfg.min_peak).collect();
        let Some(now) = now.filter(|n| n.t.is_finite() && n.lap_pos.is_finite()) else {
            return CueState { cue_zones, ..CueState::empty(CueMode::NoCar) };
        };
        if cue_zones.is_empty() || track_length <= 0.0 {
            return CueState::empty(CueMode::NoZones);
        }
        let clock = Clock { lap };
        let a = clock.at(now.lap_pos);
        if a < self.last_clock - BACKWARDS_S {
            self.reset();
            self.key = Some(key);
        }
        self.last_clock = a;
        let start = *self.start.get_or_insert(a);

        self.grade_new_brake_ons(live, &clock, &cue_zones, track_length);
        self.record_zones_without_braking(&clock, &cue_zones, a, start);
        self.state(now, a, &clock, cue_zones, track_length, cfg)
    }

    /// 1. Grades new brake-ons against their zone's reference brake point, and refreshes
    ///    the events already graded (their peak, and whether you're still braking).
    fn grade_new_brake_ons(&mut self, live: &LiveTrace, clock: &Clock, cue_zones: &[usize], track_length: f64) {
        let by_on_t: HashMap<u64, BrakeEvent> = live.events().map(|e| (e.on_t.to_bits(), *e)).collect();
        for r in self.results.values_mut() {
            if let Some(ev) = r.event.as_mut()
                && let Some(now) = by_on_t.get(&ev.on_t.to_bits())
            {
                *ev = *now;
            }
        }
        let all: Vec<usize> = (0..clock.lap.zones.len()).collect();
        for ev in live.events() {
            if ev.on_t <= self.seen_to {
                continue;
            }
            self.seen_to = ev.on_t;
            let pos = ev.on_d / track_length;
            let a = clock.at(pos);
            let Some(o) = clock.matching(a, &all) else { continue };
            if !cue_zones.contains(&o.k) || self.results.contains_key(&o.key()) {
                continue;
            }
            let dm = (pos - (o.m as f64 + clock.lap.pct[clock.lap.zones[o.k].start])) * track_length;
            self.record(Graded { m: o.m, k: o.k, dt: Some(a - o.s), dm: Some(dm), event: Some(*ev) });
        }
    }

    /// 2. Zones driven through without braking.
    fn record_zones_without_braking(&mut self, clock: &Clock, cue_zones: &[usize], a: f64, start: f64) {
        let m_now = (a / clock.lap.lap_time).floor() as i64;
        for m in m_now - 1..=m_now {
            for &k in cue_zones {
                let o = clock.occurrence(m, k);
                if o.e < a && o.s >= start && !self.results.contains_key(&o.key()) {
                    self.record(Graded { m, k, dt: None, dm: None, event: None });
                }
            }
        }
    }

    fn record(&mut self, g: Graded) {
        let key = (g.m, g.k);
        self.results.insert(key, g);
        self.order.push_back(key);
        while self.order.len() > MAX_RESULTS {
            if let Some(old) = self.order.pop_front() {
                self.results.remove(&old);
            }
        }
    }

    /// 3. The zone ahead (or the one you're still braking in) and what the window shows.
    fn state(
        &self,
        now: CuePosition,
        a: f64,
        clock: &Clock,
        cue_zones: Vec<usize>,
        track_length: f64,
        cfg: &CueConfig,
    ) -> CueState {
        let lap = clock.lap;
        let lead = cfg.lead.max(0.1);
        let grade = |dt: Option<f64>| Grade::of(dt, cfg.tol, cfg.perfect.min(cfg.tol));
        let m_now = (a / lap.lap_time).floor() as i64;
        // Still braking, or the grade still showing, keeps a zone current.
        let cur = (m_now - 1..=m_now + 1)
            .flat_map(|m| cue_zones.iter().map(move |&k| (m, k)))
            .map(|(m, k)| clock.occurrence(m, k))
            .find(|o| match self.results.get(&o.key()) {
                Some(r) => r.event.is_some_and(|e| e.active || now.t - e.on_t < FLASH_HOLD + FLASH_DRAIN),
                None => a <= o.e,
            })
            .expect("a zone lies within three lap copies");
        let z = lap.zones[cur.k];
        let r = self.results.get(&cur.key());
        let cue_at = cur.s - cfg.early;
        let arm_at = cue_at - lead;
        // After a short straight the count joins part-way (e.g. at 2), never mid-zone.
        let join_at = arm_at.max(clock.previous_end(cur.m, cur.k, &cue_zones));
        let dist = (cur.m as f64 + lap.pct[z.start] - now.lap_pos) * track_length;
        let frac = |x: f64| ((x - arm_at) / lead).clamp(0.0, 1.0) as f32;
        let zone_no = |k: usize| cue_zones.iter().position(|&c| c == k).map_or(0, |i| i + 1);

        let mut st = CueState {
            mode: CueMode::Idle,
            join: frac(join_at),
            zone_no: zone_no(cur.k),
            zone_count: cue_zones.len(),
            until_brake: cue_at - a,
            dist,
            target: Some(z.peak),
            live: now.brake.clamp(0.0, 1.0),
            ..CueState::empty(CueMode::Idle)
        };
        if let Some(r) = r {
            // Braking: the bar stops where the brake went on.
            st.mode = CueMode::Braking;
            st.fill = st.join.max(frac(cur.s + r.dt.unwrap_or(0.0)));
            if let Some(ev) = r.event {
                st.peak = Some(ev.peak);
                let since = now.t - ev.on_t;
                if since < FLASH_HOLD + FLASH_DRAIN {
                    let alpha = if since < FLASH_HOLD { 1.0 } else { 1.0 - (since - FLASH_HOLD) / FLASH_DRAIN };
                    st.flash = Some(Flash { grade: grade(r.dt), alpha: alpha.clamp(0.0, 1.0) as f32 });
                }
            }
        } else if a < join_at {
            st.mode = CueMode::Idle;
        } else if a < cue_at {
            st.mode = CueMode::Countdown;
            st.fill = frac(a);
            st.beat = (((cue_at - a) / lead) * 3.0).ceil().clamp(1.0, 3.0) as u8;
        } else {
            st.mode = CueMode::Brake;
            st.fill = 1.0;
        }

        // Timing: this zone while it's being decided, else the last one graded.
        let verdict_of = |g: &Graded, pending: bool| Verdict {
            zone_no: zone_no(g.k),
            grade: grade(g.dt),
            dt: g.dt,
            dm: g.dm,
            pending,
            current: (g.m, g.k) == cur.key(),
            peak: g.event.map(|e| e.peak),
            target: lap.zones[g.k].peak,
        };
        st.verdict = if st.mode == CueMode::Brake && a > cur.s {
            let g = Graded { m: cur.m, k: cur.k, dt: Some(a - cur.s), dm: Some(-dist), event: None };
            Some(verdict_of(&g, true))
        } else if let Some(r) = r {
            Some(verdict_of(r, false))
        } else {
            self.order.back().and_then(|k| self.results.get(k)).map(|g| verdict_of(g, false))
        };

        // Your final pressure in the zone you just left: for FINAL_HOLD after you release,
        // and only until the next countdown starts.
        if st.mode == CueMode::Idle
            && let Some(g) = self.order.back().and_then(|k| self.results.get(k))
            && let Some(ev) = g.event
            && !ev.active
            && let Some(off) = ev.off_t
            && now.t - off <= FINAL_HOLD
        {
            st.final_peak = Some(FinalPeak { peak: ev.peak, target: lap.zones[g.k].peak, age: now.t - off });
        }

        // One pip per zone: this lap's grade, else last lap's (dimmed).
        st.pips = cue_zones
            .iter()
            .map(|&k| {
                let this_lap = self.results.get(&(cur.m, k));
                let was = this_lap.or_else(|| self.results.get(&(cur.m - 1, k)));
                Pip { grade: was.map(|g| grade(g.dt)), stale: this_lap.is_none(), current: k == cur.k }
            })
            .collect();

        // Brake-ons to underline on the graph.
        st.marks = self
            .order
            .iter()
            .skip(self.order.len().saturating_sub(MARKS))
            .filter_map(|key| self.results.get(key))
            .filter_map(|g| {
                let ev = g.event?;
                Some(Mark { lap: g.m, zone: g.k, grade: grade(g.dt), on_t: ev.on_t, on_d: ev.on_d })
            })
            .collect();
        st.cue_zones = cue_zones;
        st
    }
}

/// The reference lap's clock: seconds into it, plus one lap time per lap.
struct Clock<'a> {
    lap: &'a Lap,
}

impl Clock<'_> {
    /// At a cumulative lap position (laps + lap fraction).
    fn at(&self, lap_pos: f64) -> f64 {
        let laps = lap_pos.floor();
        laps * self.lap.lap_time + self.lap.index_at_pct(lap_pos - laps) / self.lap.hz
    }

    fn occurrence(&self, m: i64, k: usize) -> Occurrence {
        let z = self.lap.zones[k];
        let base = m as f64 * self.lap.lap_time;
        Occurrence { m, k, s: base + z.start as f64 / self.lap.hz, e: base + z.end as f64 / self.lap.hz }
    }

    /// Brake-off of the zone before (m, k) among the zone indices in `list` (lap order).
    fn previous_end(&self, m: i64, k: usize, list: &[usize]) -> f64 {
        match list.iter().position(|&c| c == k) {
            Some(i) if i > 0 => self.occurrence(m, list[i - 1]).e,
            _ => self.occurrence(m - 1, *list.last().expect("a zone")).e,
        }
    }

    /// The zone a brake-on at clock `a` belongs to: the first one not yet over, if `a` is
    /// after the previous zone ended and no more than [`MATCH_BEFORE`] ahead of its brake
    /// point. All zones take part (`all`), so a light dab the countdown skips doesn't count
    /// against the next one.
    fn matching(&self, a: f64, all: &[usize]) -> Option<Occurrence> {
        let m0 = (a / self.lap.lap_time).floor() as i64;
        for m in m0 - 1..=m0 + 1 {
            for &k in all {
                let o = self.occurrence(m, k);
                if a > o.e {
                    continue;
                }
                return (a >= self.previous_end(m, k, all).max(o.s - MATCH_BEFORE)).then_some(o);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::sample_lap;
    use crate::trace::LiveSample;

    const L: f64 = 5796.1;

    #[test]
    fn grades_follow_the_windows() {
        let g = |dt: f64| Grade::of(Some(dt), 0.08, 0.03);
        assert_eq!(g(0.0), Grade::Perfect);
        assert_eq!(g(-0.03), Grade::Perfect);
        assert_eq!(g(0.05), Grade::Good);
        assert_eq!(g(-0.08), Grade::Good);
        assert_eq!(g(-0.1), Grade::Early);
        assert_eq!(g(0.24), Grade::Late);
        assert_eq!(g(-0.3), Grade::VeryEarly);
        assert_eq!(g(0.3), Grade::VeryLate);
        assert_eq!(Grade::of(None, 0.08, 0.03), Grade::NoBrake);
    }

    /// Replays the reference lap from its start for `seconds`, braking like the reference
    /// but `shift` samples later (negative: earlier), and collects the window's states.
    struct Drive {
        lap: Lap,
        live: LiveTrace,
        cue: BrakeCue,
        cfg: CueConfig,
        shift: isize,
        i: usize,
        skip: Vec<usize>,
    }

    impl Drive {
        fn new(shift: isize) -> Self {
            let lap = sample_lap();
            Self {
                lap,
                live: LiveTrace::new(),
                cue: BrakeCue::new(),
                cfg: CueConfig::default(),
                shift,
                i: 0,
                skip: vec![],
            }
        }

        /// One sample; returns the state after it.
        fn step(&mut self) -> CueState {
            let lap = &self.lap;
            let n = lap.n();
            let (laps, j) = (self.i / n, self.i % n);
            let lap_pos = laps as f64 + lap.pct[j];
            let t = self.i as f64 / lap.hz;
            let src = j as isize - self.shift;
            let in_skipped = lap
                .zones
                .iter()
                .enumerate()
                .any(|(k, z)| self.skip.contains(&k) && (z.start.saturating_sub(20)..=z.end + 20).contains(&j));
            let brake = if src < 0 || src as usize >= n || in_skipped { 0.0 } else { lap.brake[src as usize] };
            self.live.push(LiveSample { t, d: lap_pos * L, brake, throttle: 0.0 });
            self.i += 1;
            let now = CuePosition { t, lap_pos, brake };
            self.cue.update(Some(now), &self.live, Some(&self.lap), L, &self.cfg)
        }

        fn run(&mut self, seconds: f64) -> Vec<CueState> {
            let steps = (seconds * self.lap.hz) as usize;
            (0..steps).map(|_| self.step()).collect()
        }
    }

    #[test]
    fn counts_three_two_one_then_brake_and_grades_the_brake_on() {
        let mut d = Drive::new(0);
        let states = d.run(40.0);
        let modes: Vec<CueMode> = states.iter().map(|s| s.mode).collect();
        let first = modes.iter().position(|&m| m == CueMode::Countdown).unwrap();
        // The count goes 3, 2, 1 with the fill rising, then you brake on the point.
        let beats: Vec<u8> =
            states[first..].iter().take_while(|s| s.mode == CueMode::Countdown).map(|s| s.beat).collect();
        assert_eq!(beats.first(), Some(&3));
        assert_eq!(beats.last(), Some(&1));
        assert!(beats.windows(2).all(|w| w[1] <= w[0]));
        let braking = states[first..].iter().find(|s| s.mode == CueMode::Braking).unwrap();
        let v = braking.verdict.unwrap();
        assert_eq!(v.grade, Grade::Perfect, "{v:?}");
        assert!(v.current && !v.pending);
        assert_eq!(braking.flash.map(|f| f.grade), Some(Grade::Perfect));
        assert_eq!(braking.zone_no, 1);
    }

    #[test]
    fn early_and_late_brake_ons_grade_by_time() {
        // 9 samples ≈ 0.15 s at 60 Hz.
        for (shift, grade) in [(-9, Grade::Early), (9, Grade::Late), (-20, Grade::VeryEarly), (20, Grade::VeryLate)] {
            let mut d = Drive::new(shift);
            let states = d.run(40.0);
            let v = states.iter().find_map(|s| s.verdict.filter(|v| !v.pending)).unwrap();
            assert_eq!(v.grade, grade, "shift {shift}: {v:?}");
            assert_eq!(v.dt.unwrap() < 0.0, shift < 0);
            assert_eq!(v.dm.unwrap() < 0.0, shift < 0);
        }
    }

    #[test]
    fn braking_early_stops_the_bar_short() {
        let mut d = Drive::new(-20);
        let states = d.run(40.0);
        let braking = states.iter().find(|s| s.mode == CueMode::Braking).unwrap();
        assert!(braking.fill < 1.0 && braking.fill > 0.7, "{}", braking.fill);
    }

    #[test]
    fn late_shows_brake_and_counts_up_before_the_brake_on() {
        let mut d = Drive::new(20);
        let states = d.run(40.0);
        let pending: Vec<&CueState> = states.iter().filter(|s| s.verdict.is_some_and(|v| v.pending)).collect();
        assert!(!pending.is_empty());
        assert!(pending.iter().all(|s| s.mode == CueMode::Brake && s.fill == 1.0));
        // In each zone, from the brake point until you brake.
        for zone in 1..=pending.last().unwrap().zone_no {
            let dts: Vec<f64> =
                pending.iter().filter(|s| s.zone_no == zone).map(|s| s.verdict.unwrap().dt.unwrap()).collect();
            assert!(dts.windows(2).all(|w| w[1] >= w[0]), "zone {zone} counts up: {dts:?}");
        }
    }

    #[test]
    fn the_grade_shows_briefly_then_empties_and_the_final_pressure_holds_three_seconds() {
        let mut d = Drive::new(0);
        let states = d.run(60.0);
        let first = states.iter().position(|s| s.flash.is_some()).unwrap();
        let flashing = states[first..].iter().take_while(|s| s.flash.is_some()).count();
        let secs = flashing as f64 / d.lap.hz;
        assert!((secs - (FLASH_HOLD + FLASH_DRAIN)).abs() < 0.05, "{secs}");
        // Zone 1 then zone 2 follow each other closely; somewhere a final pressure shows,
        // only between zones and for at most FINAL_HOLD.
        let with_final: Vec<&CueState> = states.iter().filter(|s| s.final_peak.is_some()).collect();
        assert!(!with_final.is_empty());
        assert!(with_final.iter().all(|s| s.mode == CueMode::Idle && s.final_peak.unwrap().age <= FINAL_HOLD));
    }

    #[test]
    fn a_zone_driven_through_is_no_brake() {
        let mut d = Drive::new(0);
        d.skip = vec![0];
        let states = d.run(40.0);
        // (While it's pending, the chip counts up from the brake point.)
        let v = states.iter().find_map(|s| s.verdict.filter(|v| !v.pending)).unwrap();
        assert_eq!((v.grade, v.zone_no), (Grade::NoBrake, 1));
        assert!(states.iter().any(|s| s.pips.first().is_some_and(|p| p.grade == Some(Grade::NoBrake))));
    }

    #[test]
    fn a_short_straight_joins_the_count_part_way() {
        let lap = sample_lap();
        let cfg = CueConfig::default();
        let cue_zones: Vec<usize> = (0..lap.zones.len()).filter(|&k| lap.zones[k].peak >= cfg.min_peak).collect();
        let clock = Clock { lap: &lap };
        // Some zone starts less than 3 s after the one before it ends.
        let short = cue_zones.windows(2).any(|w| clock.occurrence(0, w[1]).s - cfg.lead < clock.occurrence(0, w[0]).e);
        assert!(short, "the sample lap has a short straight");
        let mut d = Drive::new(0);
        assert!(d.run(120.0).iter().any(|s| s.mode == CueMode::Countdown && s.join > 0.0));
    }

    #[test]
    fn brake_shows_at_the_brake_point_whatever_the_timing() {
        for shift in [-9, 0, 9] {
            let mut d = Drive::new(shift);
            let states = d.run(40.0);
            let crossing = states.windows(2).position(|w| w[0].until_brake > 0.0 && w[1].until_brake <= 0.0);
            assert!(crossing.is_some(), "shift {shift}");
        }
    }

    #[test]
    fn moving_back_on_the_clock_forgets_the_grades() {
        let mut d = Drive::new(0);
        d.run(40.0);
        assert!(!d.cue.results.is_empty());
        let state = d.cue.update(
            Some(CuePosition { t: 1000.0, lap_pos: 0.01, brake: 0.0 }),
            &LiveTrace::new(),
            Some(&d.lap),
            L,
            &d.cfg,
        );
        assert!(d.cue.results.is_empty());
        assert_eq!(state.verdict, None);
    }

    #[test]
    fn nothing_to_count_without_a_reference_or_zones() {
        let live = LiveTrace::new();
        let mut cue = BrakeCue::new();
        let now = Some(CuePosition { t: 0.0, lap_pos: 0.5, brake: 0.0 });
        assert_eq!(cue.update(now, &live, None, L, &CueConfig::default()).mode, CueMode::NoReference);
        let lap = sample_lap();
        let cfg = CueConfig { min_peak: 1.1, ..CueConfig::default() };
        assert_eq!(cue.update(now, &live, Some(&lap), L, &cfg).mode, CueMode::NoZones);
        let waiting = cue.update(None, &live, Some(&lap), L, &CueConfig::default());
        assert_eq!(waiting.mode, CueMode::NoCar);
        assert!(!waiting.cue_zones.is_empty(), "the graph still marks the zones");
    }
}
