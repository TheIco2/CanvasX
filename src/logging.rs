// logging.rs — Universal drop-in logger for ProjectOpen applications.
//
// Logs are written to:
//   ~/ProjectOpen/.Logs/<app_name>/<segment>/<date>_<app_name>_<segment>.log
//
// A new log file is created each day. The background writer thread handles
// I/O so logging never blocks the main/render thread.
//
// Implements `log::Log` so crates using `log::info!()` etc. are captured.
// Also exports `info!`, `warn!`, `error!` macros for direct use.
//
// Usage:
//   ```rust
//   mod logging;
//   // ...
//   logging::init("OpenDesktop", "Core", cfg!(debug_assertions));
//   info!("Hello from Core");
//   ```

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Sender},
        OnceLock,
    },
    thread,
};

use chrono;
use log::{Level, LevelFilter, Log, Metadata, Record};

// ---------------------------------------------------------------------------
// Global state
// ---------------------------------------------------------------------------

/// Whether debug-level messages are enabled.
static DEBUG_ENABLED: AtomicBool = AtomicBool::new(false);

/// Sender for the background writer thread.
static LOG_TX: OnceLock<Sender<String>> = OnceLock::new();

/// Singleton logger instance (required by `log::set_logger`).
static LOGGER: ProjectOpenLogger = ProjectOpenLogger;

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Initialise the logger.
///
/// - `app_name`: application name (e.g. "OpenDesktop", "OpenPeripheral").
/// - `segment`: component name (e.g. "Core", "Wallpaper", "Server").
/// - `debug`: if true, captures Debug-level and above; otherwise Warn and above.
///
/// Call once at startup. Panics if called more than once.
pub fn init(app_name: &str, segment: &str, debug: bool) {
    if LOG_TX.get().is_some() {
        panic!("logging::init() called more than once");
    }

    DEBUG_ENABLED.store(debug, Ordering::Relaxed);

    let app = app_name.to_owned();
    let seg = segment.to_owned();

    let (tx, rx) = mpsc::channel::<String>();
    LOG_TX.set(tx).expect("LOG_TX already set");

    // Background writer thread with daily rotation.
    thread::spawn(move || {
        writer_loop(&app, &seg, rx);
    });

    // Register as the global `log` crate backend.
    let max_level = if debug {
        LevelFilter::Debug
    } else {
        LevelFilter::Warn
    };

    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(max_level))
        .expect("Failed to set logger");
}

/// Returns true if debug-level logging is active.
#[inline]
pub fn enabled() -> bool {
    DEBUG_ENABLED.load(Ordering::Relaxed)
}

/// Returns true if a message at the given level should be logged.
#[inline]
pub fn should_log(level: &str) -> bool {
    if !DEBUG_ENABLED.load(Ordering::Relaxed) {
        return level == "WARN" || level == "ERROR";
    }
    true
}

/// Set debug mode at runtime.
pub fn set_debug(debug: bool) {
    DEBUG_ENABLED.store(debug, Ordering::Relaxed);
    let max_level = if debug { LevelFilter::Debug } else { LevelFilter::Warn };
    log::set_max_level(max_level);
}

/// Enqueue a log message to the background writer.
#[inline]
pub fn enqueue(level: &str, msg: String) {
    if let Some(tx) = LOG_TX.get() {
        let ts = chrono::Local::now()
            .format("%Y-%m-%d %H:%M:%S%.3f")
            .to_string();
        let _ = tx.send(format!("{ts} [{level}] {msg}"));
    }
}

// ---------------------------------------------------------------------------
// log::Log implementation
// ---------------------------------------------------------------------------

struct ProjectOpenLogger;

impl Log for ProjectOpenLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        if DEBUG_ENABLED.load(Ordering::Relaxed) {
            metadata.level() <= Level::Debug
        } else {
            metadata.level() <= Level::Warn
        }
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }

        let level = record.level();
        let msg = format!("{}", record.args());

        // Also print to stderr for immediate visibility during development.
        if DEBUG_ENABLED.load(Ordering::Relaxed) {
            eprintln!("[{level}] {msg}");
        }

        enqueue(&level.to_string(), msg);
    }

    fn flush(&self) {}
}

// ---------------------------------------------------------------------------
// Macros
// ---------------------------------------------------------------------------

#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {{
        if $crate::logging::should_log("INFO") {
            $crate::logging::enqueue("INFO", format!($($arg)*));
        }
    }};
}

#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {{
        $crate::logging::enqueue("WARN", format!($($arg)*));
    }};
}

#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {{
        $crate::logging::enqueue("ERROR", format!($($arg)*));
    }};
}

// ---------------------------------------------------------------------------
// Background writer with daily rotation
// ---------------------------------------------------------------------------

/// Resolve the logs base directory:
/// `~/ProjectOpen/.Logs/<app_name>/<segment>/`
fn logs_dir(app_name: &str, segment: &str) -> PathBuf {
    let home = std::env::var("USERPROFILE")
        .ok()
        .or_else(|| {
            let drive = std::env::var("HOMEDRIVE").ok()?;
            let path = std::env::var("HOMEPATH").ok()?;
            Some(format!("{drive}{path}"))
        })
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            // Fallback: next to exe
            std::env::current_exe()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .unwrap_or_else(|| PathBuf::from("."))
        });

    home.join("ProjectOpen")
        .join(".Logs")
        .join(app_name)
        .join(segment)
}

/// Build the log filename for a given date:
/// `<date>_<app_name>_<segment>.log`
fn log_filename(app_name: &str, segment: &str, date: &str) -> String {
    format!("{date}_{app_name}_{segment}.log")
}

/// Background writer loop. Opens a new file each day.
fn writer_loop(app_name: &str, segment: &str, rx: mpsc::Receiver<String>) {
    let dir = logs_dir(app_name, segment);
    let _ = fs::create_dir_all(&dir);

    let mut current_date = today();
    let mut file = open_log_file(&dir, app_name, segment, &current_date);

    while let Ok(line) = rx.recv() {
        let now_date = today();
        if now_date != current_date {
            // Day changed — rotate to a new file.
            current_date = now_date;
            file = open_log_file(&dir, app_name, segment, &current_date);
        }

        if let Some(ref mut f) = file {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

fn open_log_file(
    dir: &PathBuf,
    app_name: &str,
    segment: &str,
    date: &str,
) -> Option<fs::File> {
    let path = dir.join(log_filename(app_name, segment, date));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
