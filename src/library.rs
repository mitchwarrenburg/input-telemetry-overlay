//! The reference library: every Garage 61 lap you load is copied into the app folder
//! and remembered, so the right one can be picked automatically when a session's
//! track and car match.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::lap::{Lap, LapError, parse_garage61_csv};
use crate::matching::{self, MatchStatus, RefInfo};
use crate::settings::write_atomic;
use crate::telemetry::SessionInfo;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// Content hash, hex. Loading the same file twice keeps one entry.
    pub id: String,
    /// File name inside the laps folder.
    pub file: String,
    /// The name it was loaded from; its Garage 61 fields feed the lap's metadata.
    pub original_name: String,
    pub driver: Option<String>,
    pub car: Option<String>,
    pub track: Option<String>,
    /// Seconds.
    pub lap_time: f64,
    pub samples: usize,
    /// Lap length in metres (from the Speed column).
    #[serde(default)]
    pub length_m: Option<f64>,
    /// Where the lap starts, degrees.
    #[serde(default)]
    pub start_latlon: Option<(f64, f64)>,
    /// Unix seconds.
    pub added: u64,
    pub last_used: u64,
}

impl LibraryEntry {
    pub fn ref_info(&self) -> RefInfo<'_> {
        RefInfo {
            track: self.track.as_deref(),
            car: self.car.as_deref(),
            length_m: self.length_m,
            start_latlon: self.start_latlon,
        }
    }

    pub fn status(&self, session: Option<&SessionInfo>) -> MatchStatus {
        matching::status(&self.ref_info(), session)
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
#[serde(default)]
pub struct Library {
    pub laps: Vec<LibraryEntry>,
    /// The lap shown as the reference; `None` shows none.
    pub active: Option<String>,
}

#[derive(Debug)]
pub enum ImportError {
    NotCsv,
    NotText,
    Lap(LapError),
    Io(io::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::NotCsv => write!(f, "That isn't a CSV. In Garage 61, open the lap and export it as CSV."),
            ImportError::NotText => write!(f, "That file isn't text. Export the lap from Garage 61 as CSV."),
            ImportError::Lap(e) => write!(f, "{e}"),
            ImportError::Io(e) => write!(f, "Couldn't read or save the lap: {e}"),
        }
    }
}

impl std::error::Error for ImportError {}

impl From<io::Error> for ImportError {
    fn from(e: io::Error) -> Self {
        ImportError::Io(e)
    }
}

/// FNV-1a, 64-bit.
fn content_id(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl Library {
    /// `<app dir>\library.json`
    pub fn index_path(app_dir: &Path) -> PathBuf {
        app_dir.join("library.json")
    }

    /// `<app dir>\laps`
    pub fn laps_dir(app_dir: &Path) -> PathBuf {
        app_dir.join("laps")
    }

    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        write_atomic(path, serde_json::to_string_pretty(self).map_err(io::Error::other)?.as_bytes())
    }

    pub fn get(&self, id: &str) -> Option<&LibraryEntry> {
        self.laps.iter().find(|e| e.id == id)
    }

    pub fn active_entry(&self) -> Option<&LibraryEntry> {
        self.active.as_deref().and_then(|id| self.get(id))
    }

    /// Reads a CSV from disk and imports it (see [`Library::import_bytes`]).
    pub fn import(&mut self, laps_dir: &Path, src: &Path, now: u64) -> Result<(LibraryEntry, Lap), ImportError> {
        let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        if !name.to_ascii_lowercase().ends_with(".csv") {
            return Err(ImportError::NotCsv);
        }
        let bytes = std::fs::read(src)?;
        self.import_bytes(laps_dir, &name, &bytes, now)
    }

    /// Validates a lap, copies it into `laps_dir`, adds (or refreshes) its entry and
    /// makes it the active reference.
    pub fn import_bytes(
        &mut self,
        laps_dir: &Path,
        file_name: &str,
        bytes: &[u8],
        now: u64,
    ) -> Result<(LibraryEntry, Lap), ImportError> {
        let text = std::str::from_utf8(bytes).map_err(|_| ImportError::NotText)?;
        let lap = parse_garage61_csv(text, file_name).map_err(ImportError::Lap)?;
        let id = content_id(bytes);
        let file = format!("{id}.csv");
        std::fs::create_dir_all(laps_dir)?;
        let dest = laps_dir.join(&file);
        if !dest.exists() {
            write_atomic(&dest, bytes)?;
        }
        let entry = LibraryEntry {
            id: id.clone(),
            file,
            original_name: file_name.to_string(),
            driver: lap.meta.driver.clone(),
            car: lap.meta.car.clone(),
            track: lap.meta.track.clone(),
            lap_time: lap.lap_time,
            samples: lap.n(),
            length_m: lap.track_length_est,
            start_latlon: lap.start_latlon,
            added: self.get(&id).map_or(now, |e| e.added),
            last_used: now,
        };
        self.laps.retain(|e| e.id != id);
        self.laps.push(entry.clone());
        self.active = Some(id);
        Ok((entry, lap))
    }

    /// Parses a saved lap.
    pub fn load_lap(&self, laps_dir: &Path, id: &str) -> Result<Lap, ImportError> {
        let entry = self
            .get(id)
            .ok_or_else(|| ImportError::Io(io::Error::new(io::ErrorKind::NotFound, "lap not in library")))?;
        let text = std::fs::read_to_string(laps_dir.join(&entry.file))?;
        parse_garage61_csv(&text, &entry.original_name).map_err(ImportError::Lap)
    }

    /// Makes a saved lap the reference (or clears it with `None`).
    pub fn set_active(&mut self, id: Option<&str>, now: u64) {
        self.active = id.filter(|id| self.get(id).is_some()).map(str::to_string);
        if let Some(e) = self.active.clone().and_then(|id| self.laps.iter_mut().find(|e| e.id == id)) {
            e.last_used = now;
        }
    }

    /// Forgets a lap and deletes its copy.
    pub fn remove(&mut self, laps_dir: &Path, id: &str) -> io::Result<()> {
        if let Some(pos) = self.laps.iter().position(|e| e.id == id) {
            let e = self.laps.remove(pos);
            if self.active.as_deref() == Some(id) {
                self.active = None;
            }
            match std::fs::remove_file(laps_dir.join(e.file)) {
                Err(err) if err.kind() != io::ErrorKind::NotFound => return Err(err),
                _ => {}
            }
        }
        Ok(())
    }

    /// The saved lap that best fits a session: same track and car, most recently used.
    pub fn best_for(&self, session: &SessionInfo) -> Option<&LibraryEntry> {
        self.laps
            .iter()
            .filter(|e| e.status(Some(session)) == MatchStatus::Match)
            .max_by_key(|e| (e.last_used, e.added))
    }

    /// Laps for the picker: matches first, then same track, then the rest; recent first.
    pub fn sorted_for(&self, session: Option<&SessionInfo>) -> Vec<(&LibraryEntry, MatchStatus)> {
        let rank = |s: MatchStatus| match s {
            MatchStatus::Match => 0,
            MatchStatus::DifferentCar => 1,
            MatchStatus::Unknown => 2,
            MatchStatus::DifferentLayout => 3,
            MatchStatus::DifferentTrack => 4,
        };
        let mut v: Vec<_> = self.laps.iter().map(|e| (e, e.status(session))).collect();
        v.sort_by(|a, b| rank(a.1).cmp(&rank(b.1)).then(b.0.last_used.cmp(&a.0.last_used)));
        v
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = include_bytes!("../assets/sample-laps/silverstone-gp-ferrari-296-gt3.csv");
    const NAME: &str =
        "Garage 61 - Sample Lap - Ferrari 296 GT3 - Silverstone Circuit (Grand Prix) - 01.55.992 - SAMPLE.csv";

    fn session(car: &str) -> SessionInfo {
        SessionInfo {
            car_name: Some(car.into()),
            car_short_name: None,
            ..crate::demo::demo_session(&crate::demo::sample_lap())
        }
    }

    #[test]
    fn import_copies_dedupes_and_activates() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (e, lap) = lib.import_bytes(&laps, NAME, SAMPLE, 100).unwrap();
        assert_eq!(e.car.as_deref(), Some("Ferrari 296 GT3"));
        assert_eq!(lap.n(), e.samples);
        assert!(laps.join(&e.file).exists());
        assert_eq!(lib.active.as_deref(), Some(e.id.as_str()));

        let (again, _) = lib.import_bytes(&laps, NAME, SAMPLE, 200).unwrap();
        assert_eq!(lib.laps.len(), 1);
        assert_eq!((again.added, again.last_used), (100, 200));

        let reloaded = lib.load_lap(&laps, &e.id).unwrap();
        assert_eq!(reloaded.meta.driver.as_deref(), Some("Sample Lap"));

        let index = Library::index_path(dir.path());
        lib.save(&index).unwrap();
        assert_eq!(Library::load(&index), lib);

        lib.remove(&laps, &e.id).unwrap();
        assert!(lib.laps.is_empty() && lib.active.is_none());
        assert!(!laps.join(&e.file).exists());
    }

    #[test]
    fn rejects_bad_files() {
        let dir = tempfile::tempdir().unwrap();
        let mut lib = Library::default();
        let src = dir.path().join("lap.txt");
        std::fs::write(&src, "x").unwrap();
        assert!(matches!(lib.import(dir.path(), &src, 0), Err(ImportError::NotCsv)));
        assert!(matches!(lib.import_bytes(dir.path(), "a.csv", &[0xff, 0xfe, 0x00], 0), Err(ImportError::NotText)));
        assert!(matches!(lib.import_bytes(dir.path(), "a.csv", b"Speed\n1\n", 0), Err(ImportError::Lap(_))));
        assert!(lib.laps.is_empty());
    }

    #[test]
    fn picks_the_matching_lap_for_a_session() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (ferrari, _) = lib.import_bytes(&laps, NAME, SAMPLE, 10).unwrap();
        let mut other = SAMPLE.to_vec();
        other.extend_from_slice(b"\n");
        let porsche_name = NAME.replace("Ferrari 296 GT3", "Porsche 911 GT3 R (992)");
        let (porsche, _) = lib.import_bytes(&laps, &porsche_name, &other, 20).unwrap();

        assert_eq!(lib.best_for(&session("Ferrari 296 GT3")).map(|e| &e.id), Some(&ferrari.id));
        assert_eq!(lib.best_for(&session("Porsche 911 GT3 R (992)")).map(|e| &e.id), Some(&porsche.id));
        assert!(lib.best_for(&session("BMW M4 GT3")).is_none());

        let order: Vec<_> =
            lib.sorted_for(Some(&session("Ferrari 296 GT3"))).iter().map(|(e, s)| (e.id.clone(), *s)).collect();
        assert_eq!(
            order,
            vec![(ferrari.id.clone(), MatchStatus::Match), (porsche.id.clone(), MatchStatus::DifferentCar)]
        );

        lib.set_active(Some(&ferrari.id), 30);
        assert_eq!(lib.active_entry().unwrap().last_used, 30);
        lib.set_active(Some("nope"), 40);
        assert!(lib.active.is_none());
    }
}
