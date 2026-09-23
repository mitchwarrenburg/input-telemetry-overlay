//! Which lap is the reference, and whether it can be drawn for the session.

use std::path::Path;
use std::sync::Arc;

use crate::lap::Lap;
use crate::library::{ImportError, Library};
use crate::matching::{self, MatchStatus, RefInfo};
use crate::telemetry::SessionInfo;
use crate::ui::settings_panel::RefCard;

use super::live::FALLBACK_TRACK_LENGTH_M;

/// The reference lap: the library's active lap (parsed once and cached), or the
/// bundled demo lap while the demo runs without a saved lap that lines up.
#[derive(Default)]
pub struct Reference {
    saved: Option<(String, Arc<Lap>)>,
    demo: Option<Arc<Lap>>,
    /// The active lap, when its file couldn't be read or parsed.
    broken: Option<String>,
}

impl Reference {
    /// The lap drawn.
    pub fn lap(&self) -> Option<&Arc<Lap>> {
        self.demo.as_ref().or(self.saved())
    }

    /// The library's active lap, loaded, whether or not the demo lap is drawn instead.
    pub fn saved(&self) -> Option<&Arc<Lap>> {
        self.saved.as_ref().map(|(_, lap)| lap)
    }

    /// Library id of the active lap when its file couldn't be loaded.
    pub fn broken(&self) -> Option<&str> {
        self.broken.as_deref()
    }

    pub fn status(&self, session: Option<&SessionInfo>) -> Option<MatchStatus> {
        self.lap().map(|lap| status(lap, session))
    }

    /// What the settings window's Reference card shows: the chosen lap (even when the
    /// demo draws the bundled one in its place), else the bundled lap being drawn.
    pub fn card(&self, session: Option<&SessionInfo>) -> Option<RefCard<'_>> {
        match (self.saved(), &self.demo) {
            (Some(saved), Some(_)) => Some(RefCard::Waiting(saved)),
            (Some(saved), None) => Some(RefCard::Saved(saved, status(saved, session))),
            (None, Some(demo)) => Some(RefCard::Bundled(demo, status(demo, session))),
            (None, None) => None,
        }
    }

    /// Caches a lap that was just imported (and made active), so it isn't parsed again.
    pub fn set_saved(&mut self, id: String, lap: Lap) {
        self.saved = Some((id, Arc::new(lap)));
        self.broken = None;
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
        let saved_lines_up = self.saved().is_some_and(|lap| status(lap, session).aligns());
        if let Some(demo) = demo_lap
            && !saved_lines_up
        {
            self.demo = Some(Arc::clone(demo));
        }
        loaded
    }

    fn load_active(&mut self, library: &Library, laps_dir: &Path) -> Result<(), ImportError> {
        self.broken = None;
        let Some(id) = library.active.as_deref() else {
            self.saved = None;
            return Ok(());
        };
        if self.saved.as_ref().is_some_and(|(cached, _)| cached == id) {
            return Ok(());
        }
        self.saved = None;
        match library.load_lap(laps_dir, id) {
            Ok(lap) => {
                self.saved = Some((id.to_string(), Arc::new(lap)));
                Ok(())
            }
            Err(e) => {
                self.broken = Some(id.to_string());
                Err(e)
            }
        }
    }
}

fn status(lap: &Lap, session: Option<&SessionInfo>) -> MatchStatus {
    matching::status(&RefInfo::from_lap(lap), session)
}

/// Switches the library to its best lap for `session` unless the active one already
/// matches track, layout and car and loads. `broken` is a lap known not to load: it
/// never satisfies the session and is never picked. Returns whether the active lap
/// changed.
///
/// Only for a real session: the demo's pretend one mustn't change the saved choice.
pub fn auto_pick(library: &mut Library, session: &SessionInfo, broken: Option<&str>, now: u64) -> bool {
    let usable = |id: &str| Some(id) != broken;
    if library.active_entry().is_some_and(|e| usable(&e.id) && e.status(Some(session)) == MatchStatus::Match) {
        return false;
    }
    // The broken lap may well be the most recent match.
    let Some(best) = library.best_for_excluding(session, broken).map(|e| e.id.clone()) else {
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
    use crate::library::LibraryEntry;

    const SAMPLE: &[u8] = include_bytes!("../../assets/sample-laps/silverstone-gp-ferrari-296-gt3.csv");
    const NAME: &str = "Garage 61 - Me - Ferrari 296 GT3 - Silverstone Circuit (Grand Prix) - 01.56.100 - X.csv";
    const SPA_NAME: &str =
        "Garage 61 - Me - Ferrari 296 GT3 - Circuit de Spa-Francorchamps (Grand Prix Pits) - 02.17.412 - X.csv";

    /// The bundled lap is drawn in place of a saved one.
    fn draws(r: &Reference, demo: &Arc<Lap>) -> bool {
        r.lap().is_some_and(|lap| Arc::ptr_eq(lap, demo))
    }

    fn spa() -> SessionInfo {
        SessionInfo {
            track_display_name: Some("Circuit de Spa-Francorchamps".into()),
            track_config_name: Some("Grand Prix Pits".into()),
            track_length_m: Some(7004.0),
            track_latlon: Some((50.4372, 5.9714)),
            ..Default::default()
        }
    }

    /// The sample lap as a Spa lap: named for Spa, without the Lat/Lon columns that
    /// would place it at Silverstone.
    fn import_spa(lib: &mut Library, laps: &Path, now: u64) -> LibraryEntry {
        let text = std::str::from_utf8(SAMPLE).unwrap();
        let without_positions: Vec<String> = text
            .lines()
            .map(|line| line.split(',').enumerate().filter(|(i, _)| !matches!(i, 2 | 3)).map(|(_, f)| f).collect())
            .map(|fields: Vec<&str>| fields.join(","))
            .collect();
        lib.import_bytes(laps, SPA_NAME, without_positions.join("\n").as_bytes(), now).unwrap().0
    }

    #[test]
    fn follows_the_library_and_caches() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (entry, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        let mut r = Reference::default();
        r.refresh(&lib, &laps, None, None).unwrap();
        assert!(r.saved().is_some() && r.lap().is_some());

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
        assert_eq!(r.broken(), Some(entry.id.as_str()));
        lib.set_active(None, 4);
        r.refresh(&lib, &laps, None, None).unwrap();
        assert_eq!(r.broken(), None, "no longer the active lap");
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
        assert!(draws(&r, &demo) && r.saved().is_none(), "empty library: demo lap");

        let (entry, lap) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        r.set_saved(entry.id.clone(), lap);
        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert!(!draws(&r, &demo), "a saved Silverstone lap wins");

        r.refresh(&lib, &laps, Some(&spa()), Some(&demo)).unwrap();
        assert!(draws(&r, &demo) && r.saved().is_some(), "doesn't line up: demo lap, saved one kept");
        r.refresh(&lib, &laps, Some(&spa()), None).unwrap();
        assert_eq!(r.status(Some(&spa())), Some(MatchStatus::DifferentTrack), "live: shown as is");
    }

    #[test]
    fn the_card_shows_the_chosen_lap_even_behind_the_demo() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let demo = Arc::new(sample_lap());
        let session = demo_session(&demo);
        let mut lib = Library::default();
        let mut r = Reference::default();

        // No saved lap: the bundled one.
        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert!(matches!(r.card(Some(&session)), Some(RefCard::Bundled(_, MatchStatus::Match))));

        // A Spa lap chosen during the demo: the card names it, waiting for its track.
        let spa_entry = import_spa(&mut lib, &laps, 1);
        assert_eq!(spa_entry.status(Some(&session)), MatchStatus::DifferentTrack);
        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert!(draws(&r, &demo));
        match r.card(Some(&session)) {
            Some(RefCard::Waiting(lap)) => assert_eq!(lap.meta.track, spa_entry.track),
            _ => panic!("expected the waiting Spa lap"),
        }

        // Live at Spa it's the saved lap, drawn.
        r.refresh(&lib, &laps, Some(&spa()), None).unwrap();
        assert!(matches!(r.card(Some(&spa())), Some(RefCard::Saved(_, _))));

        // Removing the choice in the demo goes back to the bundled lap.
        lib.set_active(None, 2);
        r.refresh(&lib, &laps, Some(&session), Some(&demo)).unwrap();
        assert!(matches!(r.card(Some(&session)), Some(RefCard::Bundled(_, MatchStatus::Match))));
        r.refresh(&lib, &laps, Some(&spa()), None).unwrap();
        assert!(r.card(Some(&spa())).is_none(), "live without a lap: no card");
    }

    #[test]
    fn auto_pick_switches_only_when_the_active_lap_doesnt_match() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (ferrari, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        let session = demo_session(&sample_lap());
        assert!(!auto_pick(&mut lib, &session, None, 2), "already the best");

        lib.set_active(None, 3);
        assert!(auto_pick(&mut lib, &session, None, 4));
        assert_eq!(lib.active.as_deref(), Some(ferrari.id.as_str()));

        lib.set_active(None, 5);
        assert!(!auto_pick(&mut lib, &spa(), None, 6), "nothing for Spa");
        assert!(lib.active.is_none());
    }

    #[test]
    fn auto_pick_passes_over_a_lap_that_doesnt_load() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (b, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        let mut a_bytes = SAMPLE.to_vec();
        a_bytes.extend_from_slice(b"\n");
        let (a, _) = lib.import_bytes(&laps, NAME, &a_bytes, 2).unwrap();
        std::fs::remove_file(laps.join(&a.file)).unwrap();
        let session = demo_session(&sample_lap());

        // As after a restart: A is active (and the most recent) but its file is gone.
        let mut r = Reference::default();
        assert!(r.refresh(&lib, &laps, Some(&session), None).is_err());
        assert_eq!(r.broken(), Some(a.id.as_str()));
        assert!(auto_pick(&mut lib, &session, r.broken(), 3));
        assert_eq!(lib.active.as_deref(), Some(b.id.as_str()));
        r.refresh(&lib, &laps, Some(&session), None).unwrap();
        assert!(r.lap().is_some() && r.broken().is_none());
        assert!(!auto_pick(&mut lib, &session, r.broken(), 4), "B stays");
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
