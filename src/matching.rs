//! Does a reference lap belong to the current session?
//!
//! Garage 61 names tracks and cars after iRacing's Data API ("Silverstone Circuit
//! (Grand Prix)"), while the sim's session info uses different strings ("Silverstone
//! Circuit" / "Arena Grand Prix"). So the track is decided physically when possible:
//! the lap's length and where it starts, against the session's `TrackLength` and
//! start/finish position. Names are the fallback.

use crate::lap::Lap;
use crate::telemetry::SessionInfo;

/// Lap length may differ from `TrackLength` by this fraction (observed: 0.17%).
const LENGTH_TOLERANCE: f64 = 0.015;
/// Lap start may be this far from the session's start/finish position (observed: 8 m).
const START_TOLERANCE_M: f64 = 150.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MatchStatus {
    /// Same track, layout and car.
    Match,
    /// Same track and layout, different car. Still lines up.
    DifferentCar,
    /// Same venue, another layout: lap distances won't line up.
    DifferentLayout,
    DifferentTrack,
    /// Not enough information on one side.
    Unknown,
}

impl MatchStatus {
    /// Distances line up (same track and layout).
    pub fn aligns(self) -> bool {
        matches!(self, MatchStatus::Match | MatchStatus::DifferentCar)
    }
}

/// What we know about a reference lap for matching.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RefInfo<'a> {
    pub track: Option<&'a str>,
    pub car: Option<&'a str>,
    /// Metres (from the Speed column).
    pub length_m: Option<f64>,
    /// Where the lap starts, degrees.
    pub start_latlon: Option<(f64, f64)>,
}

impl<'a> RefInfo<'a> {
    pub fn from_lap(lap: &'a Lap) -> Self {
        Self {
            track: lap.meta.track.as_deref(),
            car: lap.meta.car.as_deref(),
            length_m: lap.track_length_est,
            start_latlon: lap.start_latlon,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrackMatch {
    Same,
    OtherLayout,
    Different,
    Unknown,
}

/// Great-circle distance in metres.
pub fn haversine_m((lat1, lon1): (f64, f64), (lat2, lon2): (f64, f64)) -> f64 {
    let (p1, p2) = (lat1.to_radians(), lat2.to_radians());
    let dp = p2 - p1;
    let dl = (lon2 - lon1).to_radians();
    let h = (dp / 2.0).sin().powi(2) + p1.cos() * p2.cos() * (dl / 2.0).sin().powi(2);
    2.0 * 6_371_008.8 * h.sqrt().asin()
}

fn fold_accent(c: char) -> char {
    match c {
        'à'..='å' | 'À'..='Å' => 'a',
        'ç' | 'Ç' => 'c',
        'è'..='ë' | 'È'..='Ë' => 'e',
        'ì'..='ï' | 'Ì'..='Ï' => 'i',
        'ñ' | 'Ñ' => 'n',
        'ò'..='ö' | 'ø' | 'Ò'..='Ö' | 'Ø' => 'o',
        'ù'..='ü' | 'Ù'..='Ü' => 'u',
        'ý' | 'ÿ' | 'Ý' => 'y',
        _ => c,
    }
}

/// Lowercase ASCII words: accents folded, `&` → `and`, punctuation dropped.
fn words(s: &str) -> Vec<String> {
    let cleaned: String = s
        .replace('&', " and ")
        .chars()
        .map(fold_accent)
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { ' ' })
        .collect();
    cleaned.split_whitespace().map(str::to_string).collect()
}

/// Words that differ between the two naming schemes, or that everything shares.
const TRACK_NOISE: [&str; 12] = [
    "circuit",
    "raceway",
    "international",
    "internazionale",
    "autodromo",
    "park",
    "racing",
    "course",
    "speedway",
    "the",
    "arena",
    "pits",
];

fn track_tokens(s: &str) -> Vec<String> {
    let mut out = Vec::new();
    for w in words(s) {
        if w == "gp" {
            out.extend(["grand".to_string(), "prix".to_string()]);
        } else if !TRACK_NOISE.contains(&w.as_str()) {
            out.push(w);
        }
    }
    out.sort();
    out.dedup();
    out
}

fn jaccard(a: &[String], b: &[String]) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 1.0;
    }
    let inter = a.iter().filter(|x| b.contains(x)).count() as f64;
    let union = (a.len() + b.len()) as f64 - inter;
    inter / union
}

/// Splits `Silverstone Circuit (Grand Prix)` into base and layout.
fn split_layout(track: &str) -> (&str, Option<&str>) {
    let t = track.trim();
    if let Some(open) = t.strip_suffix(')').and_then(|s| s.rfind('(')) {
        return (t[..open].trim(), Some(t[open + 1..t.len() - 1].trim()));
    }
    (t, None)
}

fn track_by_name(reference: &str, session: &SessionInfo) -> TrackMatch {
    let Some(display) = session.track_display_name.as_deref().filter(|s| !s.trim().is_empty()) else {
        return TrackMatch::Unknown;
    };
    let (base, layout) = split_layout(reference);
    if jaccard(&track_tokens(base), &track_tokens(display)) < 0.6 {
        return TrackMatch::Different;
    }
    let config = session.track_config_name.as_deref().map(track_tokens).unwrap_or_default();
    match layout.map(track_tokens) {
        // Single-layout track, or Garage 61 dropped a layout equal to the track name.
        _ if config.is_empty() => TrackMatch::Same,
        None if config.iter().all(|w| track_tokens(base).contains(w)) => TrackMatch::Same,
        Some(l) if l == config => TrackMatch::Same,
        _ => TrackMatch::OtherLayout,
    }
}

fn track_match(r: &RefInfo, session: &SessionInfo) -> TrackMatch {
    let length_ok = match (r.length_m, session.track_length_m) {
        (Some(a), Some(b)) if b > 0.0 => Some(((a - b) / b).abs() <= LENGTH_TOLERANCE),
        _ => None,
    };
    let start_ok = match (r.start_latlon, session.track_latlon) {
        (Some(a), Some(b)) => Some(haversine_m(a, b) <= START_TOLERANCE_M),
        _ => None,
    };
    let by_name = r.track.map_or(TrackMatch::Unknown, |t| track_by_name(t, session));
    match (start_ok, length_ok) {
        (Some(true), Some(true)) => TrackMatch::Same,
        (Some(true), Some(false)) => TrackMatch::OtherLayout,
        (Some(false), _) => TrackMatch::Different,
        // No positions: the name decides, and a length mismatch still rules it out.
        (None, Some(false)) if by_name == TrackMatch::Same => TrackMatch::OtherLayout,
        (None, Some(false)) => TrackMatch::Different,
        _ => by_name,
    }
}

/// Car names: equal after normalising, or one contains all the other's words
/// (covers "Dallara P217" vs "Dallara P217 LMP2").
fn car_match(reference: &str, session: &SessionInfo) -> Option<bool> {
    let want = words(reference);
    if want.is_empty() {
        return None;
    }
    let names: Vec<Vec<String>> = [&session.car_name, &session.car_short_name]
        .into_iter()
        .flatten()
        .map(|s| words(s))
        .filter(|w| !w.is_empty())
        .collect();
    if names.is_empty() {
        return None;
    }
    Some(names.iter().any(|n| n.iter().all(|w| want.contains(w)) || want.iter().all(|w| n.contains(w))))
}

/// How a reference lap relates to the session.
pub fn status(r: &RefInfo, session: Option<&SessionInfo>) -> MatchStatus {
    let Some(session) = session else { return MatchStatus::Unknown };
    match track_match(r, session) {
        TrackMatch::Unknown => MatchStatus::Unknown,
        TrackMatch::Different => MatchStatus::DifferentTrack,
        TrackMatch::OtherLayout => MatchStatus::DifferentLayout,
        TrackMatch::Same => match r.car.and_then(|c| car_match(c, session)) {
            Some(false) => MatchStatus::DifferentCar,
            _ => MatchStatus::Match,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn silverstone(car: &str) -> SessionInfo {
        SessionInfo {
            track_display_name: Some("Silverstone Circuit".into()),
            track_config_name: Some("Arena Grand Prix".into()),
            track_length_m: Some(5796.1),
            track_latlon: Some((52.068299, -1.023457)),
            car_name: Some(car.into()),
            car_short_name: Some(car.into()),
            ..Default::default()
        }
    }

    fn r<'a>(track: &'a str, car: &'a str, len: Option<f64>, start: Option<(f64, f64)>) -> RefInfo<'a> {
        RefInfo { track: Some(track), car: Some(car), length_m: len, start_latlon: start }
    }

    const START: (f64, f64) = (52.0683531, -1.0235284);

    #[test]
    fn physical_check_beats_different_names() {
        let s = silverstone("Ferrari 296 GT3");
        assert_eq!(
            status(&r("Silverstone Circuit (Grand Prix)", "Ferrari 296 GT3", Some(5786.4), Some(START)), Some(&s)),
            MatchStatus::Match
        );
        // Same start line, 3.6 km lap: another layout.
        assert_eq!(
            status(&r("Silverstone Circuit (National)", "Ferrari 296 GT3", Some(2638.0), Some(START)), Some(&s)),
            MatchStatus::DifferentLayout
        );
        // Starts 2 km away.
        assert_eq!(
            status(
                &r("Silverstone Circuit (Grand Prix)", "Ferrari 296 GT3", Some(5786.4), Some((52.08, -1.0))),
                Some(&s)
            ),
            MatchStatus::DifferentTrack
        );
        assert_eq!(
            status(
                &r("Silverstone Circuit (Grand Prix)", "Porsche 911 GT3 R (992)", Some(5786.4), Some(START)),
                Some(&s)
            ),
            MatchStatus::DifferentCar
        );
    }

    #[test]
    fn names_decide_without_positions() {
        let mut s = silverstone("Ferrari 296 GT3");
        s.track_latlon = None;
        s.track_length_m = None;
        assert_eq!(
            status(&r("Silverstone Circuit (Grand Prix)", "Ferrari 296 GT3", None, None), Some(&s)),
            MatchStatus::Match
        );
        assert_eq!(
            status(&r("Silverstone Circuit (National)", "Ferrari 296 GT3", None, None), Some(&s)),
            MatchStatus::DifferentLayout
        );
        assert_eq!(
            status(&r("Circuit de Spa-Francorchamps (Grand Prix Pits)", "Ferrari 296 GT3", None, None), Some(&s)),
            MatchStatus::DifferentTrack
        );

        let imola = SessionInfo {
            track_display_name: Some("Autodromo Enzo e Dino Ferrari".into()),
            car_name: Some("Aston Martin Valkyrie".into()),
            ..Default::default()
        };
        assert_eq!(
            status(
                &r("Autodromo Internazionale Enzo e Dino Ferrari (Grand Prix)", "Aston Martin Valkyrie", None, None),
                Some(&imola)
            ),
            MatchStatus::Match
        );

        let spa = SessionInfo {
            track_display_name: Some("Circuit de Spa-Francorchamps".into()),
            track_config_name: Some("Grand Prix".into()),
            ..Default::default()
        };
        assert_eq!(
            status(&r("Circuit de Spa-Francorchamps (Grand Prix Pits)", "x", None, None), Some(&spa)),
            MatchStatus::Match
        );

        let nurb = SessionInfo {
            track_display_name: Some("Nürburgring Combined".into()),
            track_config_name: Some("Gesamtstrecke VLN".into()),
            ..Default::default()
        };
        assert_eq!(
            status(&r("Nurburgring Combined (Gesamtstrecke VLN)", "x", None, None), Some(&nurb)),
            MatchStatus::Match
        );

        let summit = SessionInfo {
            track_display_name: Some("Summit Point Raceway".into()),
            track_config_name: Some("Summit Point Raceway".into()),
            ..Default::default()
        };
        assert_eq!(status(&r("Summit Point Raceway", "x", None, None), Some(&summit)), MatchStatus::Match);
    }

    #[test]
    fn length_alone_rules_out_other_layouts() {
        let mut s = silverstone("Ferrari 296 GT3");
        s.track_latlon = None;
        assert_eq!(
            status(&r("Silverstone Circuit (Grand Prix)", "Ferrari 296 GT3", Some(5786.4), None), Some(&s)),
            MatchStatus::Match
        );
        assert_eq!(
            status(&r("Silverstone Circuit (Grand Prix)", "Ferrari 296 GT3", Some(3600.0), None), Some(&s)),
            MatchStatus::DifferentLayout
        );
    }

    #[test]
    fn car_names_tolerate_suffixes_and_case() {
        let s = silverstone("Dallara P217 LMP2");
        assert_eq!(car_match("Dallara P217", &s), Some(true));
        let s = silverstone("Mclaren 570s GT4");
        assert_eq!(car_match("McLaren 570S GT4", &s), Some(true));
        let s = silverstone("Lamborghini Huracan GT3 EVO");
        assert_eq!(car_match("Lamborghini Huracán GT3 EVO", &s), Some(true));
        let s = silverstone("BMW M4 GT3");
        assert_eq!(car_match("BMW M4 G82 GT4 Evo", &s), Some(false));
    }

    #[test]
    fn unknown_without_a_session_or_names() {
        let s = SessionInfo::default();
        assert_eq!(status(&r("Silverstone Circuit (Grand Prix)", "x", None, None), None), MatchStatus::Unknown);
        assert_eq!(status(&RefInfo::default(), Some(&s)), MatchStatus::Unknown);
        assert!(MatchStatus::DifferentCar.aligns() && !MatchStatus::DifferentLayout.aligns());
    }

    #[test]
    fn haversine_is_sane() {
        let d = haversine_m(START, (52.068299, -1.023457));
        assert!((5.0..12.0).contains(&d), "{d}");
    }
}
