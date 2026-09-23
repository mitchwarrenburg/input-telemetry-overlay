//! The reference library: every Garage 61 lap you load is copied into the app folder
//! and remembered, so the right one can be picked automatically when a session's
//! track and car match.

use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::lap::{Lap, LapError, parse_garage61_csv};
use crate::matching::{self, MatchStatus, RefInfo};
use crate::settings::write_atomic;
use crate::telemetry::SessionInfo;

/// Largest file taken as a lap: a 60 Hz lap is 1-2 MB, a Nordschleife lap about 5 MB.
pub const MAX_LAP_FILE_BYTES: u64 = 32 * 1024 * 1024;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct LibraryEntry {
    /// Content hash, 16 lowercase hex digits. Loading the same file twice keeps one entry.
    pub id: String,
    /// File name inside the laps folder: always `<id>.csv`.
    #[serde(default)]
    pub file: String,
    /// The name it was loaded from; its Garage 61 fields feed the lap's metadata.
    pub original_name: String,
    pub driver: Option<String>,
    pub car: Option<String>,
    pub track: Option<String>,
    /// Seconds.
    pub lap_time: f64,
    #[serde(default)]
    pub samples: usize,
    /// Lap length in metres (from the Speed column).
    #[serde(default)]
    pub length_m: Option<f64>,
    /// Where the lap starts, degrees.
    #[serde(default)]
    pub start_latlon: Option<(f64, f64)>,
    /// Unix seconds.
    #[serde(default)]
    pub added: u64,
    #[serde(default)]
    pub last_used: u64,
}

impl LibraryEntry {
    /// The id is a content hash and `file` is its own `<id>.csv`: nothing a hand-edited
    /// index says can point outside the laps folder.
    fn is_valid(&self) -> bool {
        let hex = self.id.len() == 16 && self.id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'));
        hex && self.file == format!("{}.csv", self.id)
    }

    /// The lap's copy in `laps_dir`; `None` for an entry that isn't valid.
    pub fn lap_path(&self, laps_dir: &Path) -> Option<PathBuf> {
        self.is_valid().then(|| laps_dir.join(&self.file))
    }

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
    /// Over [`MAX_LAP_FILE_BYTES`].
    TooLarge,
    /// A saved copy that no longer parses and isn't what was imported.
    Damaged,
    Lap(LapError),
    Io(io::Error),
}

impl fmt::Display for ImportError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ImportError::NotCsv => write!(f, "That isn't a CSV. In Garage 61, open the lap and export it as CSV."),
            ImportError::NotText => write!(f, "That file isn't text. Export the lap from Garage 61 as CSV."),
            ImportError::TooLarge => write!(
                f,
                "That file is too big to be one lap (over {} MB). Export a single lap from Garage 61 as CSV.",
                MAX_LAP_FILE_BYTES / (1024 * 1024)
            ),
            ImportError::Damaged => {
                write!(f, "The copy in the laps folder is damaged. Load the lap's CSV again to repair it.")
            }
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

/// Reads a lap file, refusing one over [`MAX_LAP_FILE_BYTES`] before reading it.
fn read_lap_file(path: &Path) -> Result<Vec<u8>, ImportError> {
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > MAX_LAP_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_LAP_FILE_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_LAP_FILE_BYTES {
        return Err(ImportError::TooLarge);
    }
    Ok(bytes)
}

fn unix_now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
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

    /// Reads the index (see [`Library::load_with_warning`]); the warning is only logged.
    pub fn load(path: &Path) -> Self {
        Self::load_with_warning(path).0
    }

    /// Reads the index at `path`; a missing file is an empty library. Entries that can't
    /// be read are left out, and so is any whose lap file isn't its own `<id>.csv`. When
    /// something was left out, or the file couldn't be read at all, the original is kept
    /// as `library.json.bad-<unix time>` and replaced by what could be read, so no later
    /// save loses it; the warning (also logged) tells the user.
    pub fn load_with_warning(path: &Path) -> (Library, Option<String>) {
        let (library, what) = match std::fs::read(path) {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return (Library::default(), None),
            Err(e) => (Library::default(), format!("The lap library couldn't be read ({e}), so it starts empty.")),
            Ok(bytes) => match Library::parse(&bytes) {
                Ok((library, 0)) => return (library, None),
                Ok((library, 1)) => {
                    (library, "A saved lap couldn't be read from the lap library and was left out.".to_string())
                }
                Ok((library, dropped)) => {
                    (library, format!("{dropped} saved laps couldn't be read from the lap library and were left out."))
                }
                Err(e) => (Library::default(), format!("The lap library couldn't be read ({e}), so it starts empty.")),
            },
        };
        let warning = match library.quarantine(path) {
            Ok(backup) => format!("{what} The original file is kept as {backup}."),
            Err(e) => format!("{what} Backing it up failed ({e}), so the next change to the library replaces it."),
        };
        log::warn!("{}: {warning}", path.display());
        (library, Some(warning))
    }

    /// Parses an index entry by entry, so one bad entry doesn't lose the others. Returns
    /// the library and how many entries were left out.
    fn parse(bytes: &[u8]) -> serde_json::Result<(Library, usize)> {
        #[derive(Deserialize, Default)]
        #[serde(default)]
        struct Stored {
            laps: Vec<Value>,
            active: Value,
        }
        let stored: Stored = serde_json::from_slice(bytes)?;
        let total = stored.laps.len();
        let mut library = Library::default();
        for value in stored.laps {
            let Ok(mut entry) = serde_json::from_value::<LibraryEntry>(value) else { continue };
            if entry.file.is_empty() {
                entry.file = format!("{}.csv", entry.id);
            }
            if entry.is_valid() && library.get(&entry.id).is_none() {
                library.laps.push(entry);
            }
        }
        library.active = stored.active.as_str().filter(|id| library.get(id).is_some()).map(str::to_string);
        let dropped = total - library.laps.len();
        Ok((library, dropped))
    }

    /// Keeps the index at `path` as `<name>.bad-<unix time>` (copied, or moved when it
    /// can't be read) and saves this library in its place. Returns the backup's name.
    fn quarantine(&self, path: &Path) -> io::Result<String> {
        let name = path.file_name().map_or("library.json".into(), |n| n.to_string_lossy().into_owned());
        let backup = format!("{name}.bad-{}", unix_now());
        let backup_path = path.with_file_name(&backup);
        if std::fs::copy(path, &backup_path).is_err() {
            std::fs::rename(path, &backup_path)?;
        }
        if let Err(e) = self.save(path) {
            log::error!("Couldn't save {}: {e}", path.display());
        }
        Ok(backup)
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
        let bytes = read_lap_file(src)?;
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
        if bytes.len() as u64 > MAX_LAP_FILE_BYTES {
            return Err(ImportError::TooLarge);
        }
        let text = std::str::from_utf8(bytes).map_err(|_| ImportError::NotText)?;
        let lap = parse_garage61_csv(text, file_name).map_err(ImportError::Lap)?;
        let id = content_id(bytes);
        let file = format!("{id}.csv");
        std::fs::create_dir_all(laps_dir)?;
        let dest = laps_dir.join(&file);
        // Named by its content, so anything else there is a damaged copy: loading the lap
        // again repairs it.
        if std::fs::read(&dest).ok().as_deref() != Some(bytes) {
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
        let path = entry
            .lap_path(laps_dir)
            .ok_or_else(|| ImportError::Io(io::Error::new(io::ErrorKind::InvalidData, "not a library lap file")))?;
        let bytes = read_lap_file(&path)?;
        let lap = std::str::from_utf8(&bytes)
            .map_err(|_| ImportError::NotText)
            .and_then(|text| parse_garage61_csv(text, &entry.original_name).map_err(ImportError::Lap));
        // A copy that no longer parses and isn't what was imported: say how to repair it.
        lap.map_err(|e| if content_id(&bytes) == entry.id { e } else { ImportError::Damaged })
    }

    /// Makes a saved lap the reference (or clears it with `None`).
    pub fn set_active(&mut self, id: Option<&str>, now: u64) {
        self.active = id.filter(|id| self.get(id).is_some()).map(str::to_string);
        if let Some(e) = self.active.clone().and_then(|id| self.laps.iter_mut().find(|e| e.id == id)) {
            e.last_used = now;
        }
    }

    /// Forgets a lap and deletes its copy (only ever its own `<id>.csv` in `laps_dir`).
    pub fn remove(&mut self, laps_dir: &Path, id: &str) -> io::Result<()> {
        if let Some(pos) = self.laps.iter().position(|e| e.id == id) {
            let e = self.laps.remove(pos);
            if self.active.as_deref() == Some(id) {
                self.active = None;
            }
            match e.lap_path(laps_dir).map(std::fs::remove_file) {
                Some(Err(err)) if err.kind() != io::ErrorKind::NotFound => return Err(err),
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

    /// The `library.json.bad-*` backups in `dir`.
    fn backups(dir: &Path) -> Vec<PathBuf> {
        let is_backup =
            |p: &PathBuf| p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("library.json.bad-"));
        std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().path()).filter(is_backup).collect()
    }

    #[test]
    fn a_damaged_index_keeps_what_it_can_and_the_original() {
        let dir = tempfile::tempdir().unwrap();
        let (laps, index) = (Library::laps_dir(dir.path()), Library::index_path(dir.path()));
        let mut lib = Library::default();
        let (ferrari, _) = lib.import_bytes(&laps, NAME, SAMPLE, 10).unwrap();
        let mut other = SAMPLE.to_vec();
        other.extend_from_slice(b"\n");
        lib.import_bytes(&laps, &NAME.replace("Ferrari 296 GT3", "BMW M4 GT3"), &other, 20).unwrap();
        lib.save(&index).unwrap();
        assert_eq!(Library::load_with_warning(&index), (lib.clone(), None));

        // A hand edit breaks the (active) BMW lap, and an entry lost its fields.
        let mut stored: Value = serde_json::from_slice(&std::fs::read(&index).unwrap()).unwrap();
        stored["laps"][1]["lap_time"] = "1:55.9".into();
        stored["laps"].as_array_mut().unwrap().push(serde_json::json!({ "id": "0123456789abcdef" }));
        let broken = serde_json::to_vec_pretty(&stored).unwrap();
        std::fs::write(&index, &broken).unwrap();

        let (loaded, warning) = Library::load_with_warning(&index);
        assert_eq!(loaded, Library { laps: vec![ferrari], active: None });
        let warning = warning.unwrap();
        assert!(warning.starts_with("2 saved laps couldn't be read"), "{warning}");
        let kept = backups(dir.path());
        assert_eq!(kept.len(), 1);
        assert!(warning.contains(&*kept[0].file_name().unwrap().to_string_lossy()), "{warning}");
        assert_eq!(std::fs::read(&kept[0]).unwrap(), broken);
        // What was read replaced the index, so the next start is quiet.
        assert_eq!(Library::load_with_warning(&index), (loaded, None));
    }

    #[test]
    fn an_unreadable_index_starts_empty_but_is_kept() {
        let dir = tempfile::tempdir().unwrap();
        let index = Library::index_path(dir.path());
        assert_eq!(Library::load_with_warning(&index), (Library::default(), None), "missing: a first start");

        // Zero-filled, as NTFS can leave a file after a power loss.
        std::fs::write(&index, [0u8; 300]).unwrap();
        let (lib, warning) = Library::load_with_warning(&index);
        assert_eq!(lib, Library::default());
        assert!(warning.unwrap().starts_with("The lap library couldn't be read"));
        assert_eq!(std::fs::read(&backups(dir.path())[0]).unwrap(), [0u8; 300]);
        assert_eq!(Library::load_with_warning(&index), (Library::default(), None));
    }

    #[test]
    fn lap_files_stay_inside_the_laps_folder() {
        let dir = tempfile::tempdir().unwrap();
        let (laps, index) = (Library::laps_dir(dir.path()), Library::index_path(dir.path()));
        let victim = dir.path().join("victim.csv");
        std::fs::write(&victim, "keep me").unwrap();
        let mut lib = Library::default();
        let (entry, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();

        // An index naming files outside the laps folder: those entries are left out.
        let mut stored = serde_json::to_value(&lib).unwrap();
        for (id, file) in
            [("0123456789abcdef", victim.to_string_lossy().into_owned()), ("fedcba9876543210", "../victim.csv".into())]
        {
            let mut evil = stored["laps"][0].clone();
            evil["id"] = id.into();
            evil["file"] = file.into();
            stored["laps"].as_array_mut().unwrap().push(evil);
        }
        std::fs::write(&index, serde_json::to_vec(&stored).unwrap()).unwrap();
        let (mut loaded, warning) = Library::load_with_warning(&index);
        assert_eq!(loaded.laps, std::slice::from_ref(&entry));
        assert!(warning.unwrap().starts_with("2 saved laps"));

        // Entries made some other way are checked where the file is used.
        for (id, file) in [("0123456789abcdef", "../victim.csv"), ("../victim", "../victim.csv"), ("x", "x.csv")] {
            let bad = LibraryEntry { id: id.into(), file: file.into(), ..entry.clone() };
            assert_eq!(bad.lap_path(&laps), None, "{id} {file}");
            loaded.laps.push(bad);
            assert!(loaded.load_lap(&laps, id).is_err());
            loaded.remove(&laps, id).unwrap();
        }
        assert!(victim.exists());
        assert_eq!(entry.lap_path(&laps), Some(laps.join(format!("{}.csv", entry.id))));
    }

    #[test]
    fn loading_a_lap_again_repairs_its_copy() {
        let dir = tempfile::tempdir().unwrap();
        let laps = Library::laps_dir(dir.path());
        let mut lib = Library::default();
        let (entry, _) = lib.import_bytes(&laps, NAME, SAMPLE, 1).unwrap();
        // Changed but still a lap (line endings rewritten by a sync tool): still loads.
        let crlf = String::from_utf8(SAMPLE.to_vec()).unwrap().replace('\n', "\r\n");
        std::fs::write(laps.join(&entry.file), crlf).unwrap();
        assert_eq!(lib.load_lap(&laps, &entry.id).unwrap().n(), entry.samples);
        std::fs::write(laps.join(&entry.file), [0u8; 100]).unwrap();
        assert!(matches!(lib.load_lap(&laps, &entry.id), Err(ImportError::Damaged)));
        lib.import_bytes(&laps, NAME, SAMPLE, 2).unwrap();
        assert_eq!(lib.load_lap(&laps, &entry.id).unwrap().n(), entry.samples);
    }

    #[test]
    fn oversized_files_are_refused_before_reading() {
        let dir = tempfile::tempdir().unwrap();
        let src = dir.path().join("stint.csv");
        std::fs::File::create(&src).unwrap().set_len(MAX_LAP_FILE_BYTES + 1).unwrap();
        let mut lib = Library::default();
        let err = lib.import(dir.path(), &src, 0).unwrap_err();
        assert!(matches!(err, ImportError::TooLarge));
        assert!(err.to_string().starts_with("That file is too big to be one lap (over 32 MB)."));
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
