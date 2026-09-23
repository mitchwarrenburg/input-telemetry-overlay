#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Starts the overlay: command line, logging, then the app.

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    start::main()
}

#[cfg(not(windows))]
fn main() {
    eprintln!("Input Telemetry Overlay runs on Windows.");
    std::process::exit(1);
}

#[cfg(windows)]
mod start {
    use std::ffi::{OsStr, OsString};
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::sync::OnceLock;

    use ito::app::LaunchOptions;
    use ito::platform::{self, Instance};
    use ito::settings::{self, SettingsTab};

    const USAGE: &str = "\
Input Telemetry Overlay: your throttle and brake against a Garage 61 reference lap.

Usage: input-telemetry-overlay [options] [lap.csv ...]

CSV files given on the command line (or dropped on the program) are added to the
lap library, and the last one becomes the reference. When the overlay is already
running for the same data folder, it takes them instead.

Options:
  --demo                       Drive the simulated car, even if iRacing is running
  --settings <tab>             Open the settings on a tab: display, labels, timing, reference
  --screenshot <png>           Save the overlay as a PNG after a second, then quit
  --settings-screenshot <png>  Save the settings window as a PNG too (opens it), then quit
  --data-dir <dir>             Keep settings and laps here instead of %APPDATA%\\input-telemetry-overlay
  -h, --help                   Show this help";

    pub fn main() -> ExitCode {
        let args: Vec<OsString> = std::env::args_os().skip(1).collect();
        let opts = match parse_args(args.iter().cloned()) {
            Ok(Command::Run(opts)) => opts,
            Ok(Command::Help) => {
                report(USAGE, false);
                return ExitCode::SUCCESS;
            }
            Err(e) => {
                logger::init(&data_dir_arg(&args).unwrap_or_else(settings::app_dir));
                log::error!("{e}");
                report(&format!("{e}\n\n{USAGE}"), true);
                return ExitCode::from(2);
            }
        };
        logger::init(&opts.data_dir());
        let instance = match Instance::claim(&opts.data_dir()) {
            Ok(Some(instance)) => Some(instance),
            Ok(None) => return hand_over(&opts),
            Err(e) => {
                log::warn!("Couldn't check for an overlay already running: {e}");
                None
            }
        };
        // `run` logs its error.
        match ito::app::run(opts, instance) {
            Ok(()) => ExitCode::SUCCESS,
            Err(_) => ExitCode::FAILURE,
        }
    }

    /// An overlay already runs for this data folder: it takes this launch's laps (or
    /// shows its settings), and this launch ends.
    fn hand_over(opts: &LaunchOptions) -> ExitCode {
        match platform::hand_over(&opts.data_dir(), &opts.import) {
            Ok(()) => {
                log::info!("Handed {:?} to the overlay already running", opts.import);
                if console() {
                    let what = match opts.import.len() {
                        0 => "showed its settings".to_owned(),
                        1 => "gave it the lap".to_owned(),
                        n => format!("gave it the {n} laps"),
                    };
                    println!("Input Telemetry Overlay is already running for this data folder: {what}.");
                }
                ExitCode::SUCCESS
            }
            Err(e) => {
                let message = format!("Couldn't reach the overlay that's already running: {e}");
                log::error!("{message}");
                report(&message, true);
                ExitCode::FAILURE
            }
        }
    }

    /// Shows help or an error on the console the program was started from, or else in
    /// a message box (e.g. started from Explorer).
    fn report(text: &str, error: bool) {
        if !console() {
            platform::message_box("Input Telemetry Overlay", text, error);
        } else if error {
            eprintln!("{text}");
        } else {
            println!("{text}");
        }
    }

    /// There's a console to print to. A release build is a GUI program without one of
    /// its own: it borrows the console it was started from, if any.
    fn console() -> bool {
        static ATTACHED: OnceLock<bool> = OnceLock::new();
        cfg!(debug_assertions) || *ATTACHED.get_or_init(platform::attach_parent_console)
    }

    #[derive(Debug)]
    enum Command {
        Run(LaunchOptions),
        Help,
    }

    /// Options are matched as UTF-8; anything else is a lap to import (any file name
    /// Windows allows) or, where an option or a tab is expected, an error.
    fn parse_args(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
        let mut opts = LaunchOptions::default();
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let shown = arg.to_string_lossy().into_owned();
            let mut value = || args.next().ok_or_else(|| format!("{shown} needs a value"));
            match arg.to_str() {
                Some("-h" | "--help") => return Ok(Command::Help),
                Some("--demo") => opts.demo = true,
                Some("--settings") => opts.open_settings = Some(parse_tab(&value()?)?),
                Some("--screenshot") => opts.screenshot = Some(value()?.into()),
                Some("--settings-screenshot") => opts.settings_screenshot = Some(value()?.into()),
                Some("--data-dir") => opts.data_dir = Some(value()?.into()),
                _ if shown.starts_with('-') => return Err(format!("Unknown option: {shown}")),
                _ => opts.import.push(arg.into()),
            }
        }
        Ok(Command::Run(opts))
    }

    fn parse_tab(name: &OsStr) -> Result<SettingsTab, String> {
        match name.to_str().map(str::to_ascii_lowercase).as_deref() {
            Some("display") => Ok(SettingsTab::Display),
            Some("labels") => Ok(SettingsTab::Labels),
            Some("timing") => Ok(SettingsTab::Timing),
            Some("reference") => Ok(SettingsTab::Reference),
            _ => Err(format!("No settings tab called “{}”", name.to_string_lossy())),
        }
    }

    /// `--data-dir`'s value, so a mistake elsewhere on the command line is logged there.
    fn data_dir_arg(args: &[OsString]) -> Option<PathBuf> {
        args.windows(2).find(|pair| pair[0] == "--data-dir").map(|pair| PathBuf::from(&pair[1]))
    }

    /// A small `log` backend. Debug builds: this app's messages (and libraries'
    /// warnings) on stderr. Release builds: warnings and errors appended to
    /// `<data dir>\overlay.log`.
    mod logger {
        use std::fs::{File, OpenOptions};
        use std::io::{self, Write};
        use std::path::Path;
        use std::sync::Mutex;
        use std::time::{SystemTime, UNIX_EPOCH};

        use log::{LevelFilter, Metadata, Record};

        /// The log file starts over when it grows past this.
        const MAX_LOG_BYTES: u64 = 1 << 20;

        struct Logger {
            out: Mutex<Box<dyn Write + Send>>,
            /// Level for this app's own messages; libraries log warnings and errors only.
            ours: LevelFilter,
        }

        impl Logger {
            fn level_for(&self, target: &str) -> LevelFilter {
                let ours =
                    target == "ito" || target.starts_with("ito::") || target.starts_with("input_telemetry_overlay");
                if ours { self.ours } else { LevelFilter::Warn.min(self.ours) }
            }
        }

        impl log::Log for Logger {
            fn enabled(&self, metadata: &Metadata) -> bool {
                metadata.level() <= self.level_for(metadata.target())
            }

            fn log(&self, record: &Record) {
                if !self.enabled(record.metadata()) {
                    return;
                }
                let t = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default();
                if let Ok(mut out) = self.out.lock() {
                    let _ = writeln!(
                        out,
                        "{}.{:03} {:<5} {}: {}",
                        t.as_secs(),
                        t.subsec_millis(),
                        record.level(),
                        record.target(),
                        record.args()
                    );
                }
            }

            fn flush(&self) {
                if let Ok(mut out) = self.out.lock() {
                    let _ = out.flush();
                }
            }
        }

        pub fn init(data_dir: &Path) {
            let (out, ours): (Box<dyn Write + Send>, _) = if cfg!(debug_assertions) {
                (Box::new(io::stderr()), LevelFilter::Debug)
            } else {
                match open_log_file(data_dir) {
                    Ok(file) => (Box::new(file), LevelFilter::Warn),
                    Err(_) => return, // nowhere to write
                }
            };
            if log::set_boxed_logger(Box::new(Logger { out: Mutex::new(out), ours })).is_ok() {
                log::set_max_level(ours);
            }
        }

        fn open_log_file(dir: &Path) -> io::Result<File> {
            std::fs::create_dir_all(dir)?;
            let path = dir.join("overlay.log");
            let too_big = std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_LOG_BYTES);
            OpenOptions::new().create(true).append(!too_big).write(true).truncate(too_big).open(path)
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn parse(args: &[&str]) -> Result<Command, String> {
            parse_args(args.iter().map(OsString::from))
        }

        /// `lap<unpaired surrogate>.csv`: a legal Windows file name that isn't Unicode.
        fn not_unicode(prefix: &str) -> OsString {
            use std::os::windows::ffi::OsStringExt;
            let mut wide: Vec<u16> = prefix.encode_utf16().collect();
            wide.push(0xD800);
            wide.extend(".csv".encode_utf16());
            OsString::from_wide(&wide)
        }

        #[test]
        fn parses_every_option() {
            let Ok(Command::Run(o)) = parse(&[
                "--demo",
                "--settings",
                "Reference",
                "--screenshot",
                "a.png",
                "--settings-screenshot",
                "b.png",
                "--data-dir",
                "d",
            ]) else {
                panic!()
            };
            assert!(o.demo);
            assert_eq!(o.open_settings, Some(SettingsTab::Reference));
            assert_eq!(o.screenshot.as_deref(), Some("a.png".as_ref()));
            assert_eq!(o.settings_screenshot.as_deref(), Some("b.png".as_ref()));
            assert_eq!(o.data_dir.as_deref(), Some("d".as_ref()));
        }

        #[test]
        fn rejects_bad_input() {
            assert!(matches!(parse(&[]), Ok(Command::Run(_))));
            assert!(matches!(parse(&["--demo", "-h"]), Ok(Command::Help)));
            assert_eq!(parse(&["--settings"]).unwrap_err(), "--settings needs a value");
            assert!(parse(&["--settings", "audio"]).is_err());
            assert!(parse(&["--fast"]).is_err());
        }

        #[test]
        fn other_arguments_are_laps_to_import() {
            let Ok(Command::Run(o)) = parse(&["a.csv", "--demo", r"C:\laps\b.csv"]) else { panic!() };
            assert_eq!(o.import, [PathBuf::from("a.csv"), r"C:\laps\b.csv".into()]);
            assert!(o.demo);
        }

        #[test]
        fn non_unicode_arguments_are_paths_or_errors() {
            let lap = not_unicode("lap");
            let Ok(Command::Run(o)) = parse_args([lap.clone(), "--data-dir".into(), not_unicode("dir")]) else {
                panic!()
            };
            assert_eq!(o.import, [PathBuf::from(&lap)]);
            assert_eq!(o.data_dir, Some(PathBuf::from(not_unicode("dir"))));

            let tab = parse_args(["--settings".into(), lap]).unwrap_err();
            assert!(tab.starts_with("No settings tab called"), "{tab}");
            let option = parse_args([not_unicode("--x")]).unwrap_err();
            assert!(option.starts_with("Unknown option: --x"), "{option}");
        }

        #[test]
        fn finds_the_data_dir_for_logging_a_bad_command_line() {
            let args: Vec<OsString> = ["--settings", "audio", "--data-dir", "d"].map(OsString::from).into();
            assert_eq!(data_dir_arg(&args), Some(PathBuf::from("d")));
            assert_eq!(data_dir_arg(&args[..3]), None);
        }
    }
}
