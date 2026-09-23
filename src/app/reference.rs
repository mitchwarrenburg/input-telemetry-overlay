//! Which lap is the reference, and whether it can be drawn for the session.

use std::path::Path;
use std::sync::Arc;

use crate::lap::Lap;
use crate::library::{ImportError, Library};
use crate::matching::{self, MatchStatus, RefInfo};
use crate::telemetry::SessionInfo;

use super::live::FALLBACK_TRACK_LENGTH_M;

/// The reference lap: the library's active lap (parsed once and cached), or the
/// bundled demo lap while the demo runs without a saved lap that lines up.
#[derive(Default)]
pub struct Reference {
    saved: Option<(String, Arc<Lap>)>,
    demo: Option<Arc<Lap>>,
}

impl Reference {
    pub fn lap(&self) -> Option<&Arc<Lap>> {
        self.demo.as_ref().or(self.saved.as_ref().map(|(_, lap)| lap))
    }

    /// Library id of the lap shown; `None` for the demo lap or no reference.
    pub fn id(&self) -> Option<&str> {
        match (&self.demo, &self.saved) {
            (None, Some((id, _))) => Some(id),
            _ => None,
        }
    }

    pub fn status(&self, session: Option<&SessionInfo>) -> Option<MatchStatus> {
        self.lap().map(|lap| matching::status(&RefInfo::from_lap(lap), session))
    }

    /// Caches a lap that was just imported (and made active), so it isn't parsed again.
    pub fn set_saved(&mut self, id: String, lap: Lap) {
        self.saved = Some((id, Arc::new(lap)));
    }

    /// Follows the library's active lap, loading it if needed. With `demo_lap`, shows
    /// that instead when the saved lap doesn't line up with `session`.
    pub fn refresh(
        &mut self,
        library: &Library,
        laps_dir: &Path,
        session: Option<&SessionInfo>,
        demo_lap: Option<&Arc<Lap>>,
    ) -> Result<(), ImportError> {
        let loaded = self.load_active(library, laps_dir);
        self.demo = None;
        if let Some(demo) = demo_lap
            && !self.status(session).is_some_and(MatchStatus::aligns)
        {
            self.demo = Some(Arc::clone(demo));
        }
        loaded
    }

    fn load_active(&mut self, library: &Library, laps_dir: &Path) -> Result<(), ImportError> {
        let Some(id) = library.active.as_deref() else {
            self.saved = None;
            return Ok(());
        };
        if self.saved.as_ref().is_some_and(|(cached, _)| cached == id) {
            return Ok(());
        }
        self.saved = None;
        let lap = library.load_lap(laps_dir, id)?;
        self.saved = Some((id.to_string(), Arc::new(lap)));
        Ok(())
    }
}

/// Switches the library to its best lap for `session` unless the active one already
/// matches track, layout and car. Returns whether the active lap changed.
pub fn auto_pick(library: &mut Library, session: &SessionInfo, now: u64) -> bool {
    if library.active_entry().is_some_and(|e| e.status(Some(session)) == MatchStatus::Match) {
        return false;
    }
    let Some(best) = library.best_for(session).map(|e| e.id.clone()) else {
        return false;
    };
    library.set_active(Some(&best), now);
    true
}

/// Whether a reference with this status is drawn, and the header badge when it isn't
/// because it belongs elsewhere.
pub fn visibility(status: MatchStatus) -> (bool, Option<&'static str>) {
    match status {
        MatchStatus::Match | MatchStatus::DifferentCar | MatchStatus::Unknown => (true, None),
        MatchStatus::DifferentLayout => (false, Some("REF: OTHER LAYOUT")),
        MatchStatus::DifferentTrack => (false, Some("REF: OTHER TRACK")),
    }
}

/// Metres per lap: the session's `TrackLength`, else the reference's estimate.
pub fn track_length(session: Option<&SessionInfo>, reference: Option<&Lap>) -> f64 {
    session
        .and_then(|s| s.track_length_m)
        .or_else(|| reference.and_then(|lap| lap.track_length_est))
        .unwrap_or(FALLBACK_TRACK_LENGTH_M)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo::{demo_session, sample_lap};

    const SAMPLE: &[u8] = include_bytes!("../../assets/sample-laps/silverstone-gp-ferrari-296-gt3.csv");
    const NAME: &str = "Garage 61 - Me - Ferrari 296 GT3 - Silverstone Circuit (Grand Prix) - 01.56.100 - X.csv";

    fn spa() -> SessionInfo {
        SessionInfo {
            track_display_name: Some("Circuit de Spa-Francorchamps".into()),
            track_config_name: Some("Grand Prix Pits".into()),
            track_length_m: Some(7004.0),
            track_latlon: Some((50.4372, 5.9714)),
            ..Default::default()
        }
    }

    #[test]
    fn follows_the_library_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (entry, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        let mut r = Reference::default();
        r.refresh(&lib, &laps, None, None).unwrap();
        assert_eq!(r.id(), Some(entry.id.as_str()));

        // Cached: works even with the file gone.
        std::fs::remove_file(laps.join(&entry.file)).unwrap();
        r.refresh(&lib, &laps, None, None).unwrap();
        assert!(r.lap().is_some());

        lib.set_active(None, 2);
        r.refresh(&lib, &laps, None, None).unwrap();
        assert!(r.lap().is_none());

        lib.set_active(Some(&entry.id), 3);
        assert!(r.refresh(&lib, &laps, None, None).is_err(), "missing file");
        assert!(r.lap().is_none());
    }

    #[test]
    fn demo_uses_the_bundled_lap_unless_a_saved_one_lines_up() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let demo = Arc::new(sample_lap());
        let session = demo_session(&demo);
        let mut lib = Library::default();
        let mut r = Reference::default();

        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert!(r.lap().is_some() && r.id().is_none(), "empty library: demo lap");

        let (entry, lap) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        r.set_saved(entry.id.clone(), lap);
        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert_eq!(r.id(), Some(entry.id.as_str()), "a saved Silverstone lap wins");

        r.refresh(&lib, &laps, Some(&spa()), Some(&demo)).unwrap();
        assert!(r.id().is_none(), "doesn't line up: demo lap");
        r.refresh(&lib, &laps, Some(&spa()), None).unwrap();
        assert_eq!(r.status(Some(&spa())), Some(MatchStatus::DifferentTrack), "live: shown as is");
    }

    #[test]
    fn auto_pick_switches_only_when_the_active_lap_doesnt_match() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (ferrari, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        let session = demo_session(&sample_lap());
        assert!(!auto_pick(&mut lib, &session, 2), "already the best");

        lib.set_active(None, 3);
        assert!(auto_pick(&mut lib, &session, 4));
        assert_eq!(lib.active.as_deref(), Some(ferrari.id.as_str()));

        lib.set_active(None, 5);
        assert!(!auto_pick(&mut lib, &spa(), 6), "nothing for Spa");
        assert!(lib.active.is_none());
    }

    #[test]
    fn laps_for_other_tracks_are_hidden_with_a_badge() {
        assert_eq!(visibility(MatchStatus::DifferentCar), (true, None));
        assert_eq!(visibility(MatchStatus::Unknown), (true, None));
        assert_eq!(visibility(MatchStatus::DifferentTrack), (false, Some("REF: OTHER TRACK")));
        assert_eq!(visibility(MatchStatus::DifferentLayout), (false, Some("REF: OTHER LAYOUT")));
    }

    #[test]
    fn track_length_prefers_the_session() {
        let lap = sample_lap();
        assert_eq!(track_length(Some(&spa()), Some(&lap)), 7004.0);
        assert_eq!(track_length(None, Some(&lap)), lap.track_length_est.unwrap());
        assert_eq!(track_length(Some(&SessionInfo::default()), None), FALLBACK_TRACK_LENGTH_M);
    }
}
