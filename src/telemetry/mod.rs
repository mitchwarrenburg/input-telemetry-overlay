//! Telemetry sources. The overlay consumes [`TelemetryEvent`]s; iRacing and the demo
//! driver both produce them.

pub mod iracing;

/// One telemetry frame (60 Hz from iRacing).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TelemetryFrame {
    /// `SessionTime`, seconds.
    pub session_time: f64,
    /// `LapDistPct`, 0..1 (negative when not on track).
    pub lap_dist_pct: f64,
    /// `Throttle`, 0..1.
    pub throttle: f32,
    /// `Brake`, 0..1.
    pub brake: f32,
    /// `IsOnTrack`: the player's car is in the world.
    pub on_track: bool,
}

/// The parts of iRacing's session info the overlay uses.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionInfo {
    /// `WeekendInfo.TrackName`, e.g. `silverstone 2019 gp`.
    pub track_name: Option<String>,
    /// `WeekendInfo.TrackDisplayName`, e.g. `Silverstone Circuit`.
    pub track_display_name: Option<String>,
    /// `WeekendInfo.TrackConfigName`, e.g. `Grand Prix`.
    pub track_config_name: Option<String>,
    /// `WeekendInfo.TrackLength` in metres. `LapDistPct` is relative to this one,
    /// not `TrackLengthOfficial`.
    pub track_length_m: Option<f64>,
    /// `WeekendInfo.TrackLatitude` / `TrackLongitude`: the start/finish line, degrees.
    pub track_latlon: Option<(f64, f64)>,
    /// Player car's `CarScreenName`, e.g. `Ferrari 296 GT3`.
    pub car_name: Option<String>,
    /// Player car's `CarScreenNameShort`.
    pub car_short_name: Option<String>,
}

impl SessionInfo {
    /// Track name the way Garage 61 writes it: `Silverstone Circuit (Grand Prix)`.
    pub fn full_track_name(&self) -> Option<String> {
        let display = self.track_display_name.as_deref().map(str::trim).filter(|s| !s.is_empty())?;
        match self.track_config_name.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            Some(cfg) => Some(format!("{display} ({cfg})")),
            None => Some(display.to_string()),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TelemetryEvent {
    /// Connected to a running sim.
    Connected,
    /// The sim closed or stopped sending data.
    Disconnected,
    /// Session info arrived or changed.
    Session(SessionInfo),
    Frame(TelemetryFrame),
}

/// Parses `TrackLatitude`-style values: iRacing writes degrees with an "m" unit
/// (`52.068299 m`).
pub fn parse_degrees(s: &str) -> Option<f64> {
    let v: f64 = s.trim().trim_end_matches('m').trim().parse().ok()?;
    (v.is_finite() && v.abs() <= 180.0).then_some(v)
}

/// Parses iRacing's `TrackLength` string (`"5.89 km"`, `"3.66 mi"`) into metres.
pub fn parse_track_length(s: &str) -> Option<f64> {
    let s = s.trim();
    let split = s.find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',')).unwrap_or(s.len());
    let value: f64 = s[..split].replace(',', ".").parse().ok()?;
    let unit = s[split..].trim().to_ascii_lowercase();
    let metres = match unit.as_str() {
        "km" | "" => value * 1000.0,
        "mi" | "miles" => value * 1609.344,
        "m" => value,
        _ => return None,
    };
    (metres > 0.0 && metres.is_finite()).then_some(metres)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_track_lengths() {
        assert_eq!(parse_track_length("5.89 km"), Some(5890.0));
        assert!((parse_track_length("3.66 mi").unwrap() - 5890.199).abs() < 0.01);
        assert_eq!(parse_track_length(" 4,01 km"), Some(4010.0));
        assert_eq!(parse_track_length("abc"), None);
        assert_eq!(parse_track_length("0 km"), None);
        assert_eq!(parse_track_length("5.7961 km"), Some(5796.1));
    }

    #[test]
    fn parses_degrees() {
        assert_eq!(parse_degrees("52.068299 m"), Some(52.068299));
        assert_eq!(parse_degrees("-1.023457 m"), Some(-1.023457));
        assert_eq!(parse_degrees("north"), None);
    }

    #[test]
    fn full_track_name_matches_garage61_style() {
        let mut s = SessionInfo {
            track_display_name: Some("Silverstone Circuit".into()),
            track_config_name: Some("Grand Prix".into()),
            ..Default::default()
        };
        assert_eq!(s.full_track_name().as_deref(), Some("Silverstone Circuit (Grand Prix)"));
        s.track_config_name = Some(" ".into());
        assert_eq!(s.full_track_name().as_deref(), Some("Silverstone Circuit"));
        s.track_display_name = None;
        assert_eq!(s.full_track_name(), None);
    }
}
