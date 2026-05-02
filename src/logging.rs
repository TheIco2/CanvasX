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

/// Whether stderr is attached to a terminal (computed once at first log).
/// When true, all log records are mirrored to stderr regardless of debug mode
/// so apps launched from a terminal show interactions live.
static STDERR_IS_TTY: OnceLock<bool> = OnceLock::new();

fn stderr_is_tty() -> bool {
    *STDERR_IS_TTY.get_or_init(|| {
        use std::io::IsTerminal;
        std::io::stderr().is_terminal()
    })
}

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
pub fn init_with_path(app_name: &str, segment: &str, debug: bool, log_dir: Option<&str>, log_level: Option<&str>) {
    if LOG_TX.get().is_some() {
        panic!("logging::init() called more than once");
    }

    DEBUG_ENABLED.store(debug, Ordering::Relaxed);

    let app = app_name.to_owned();
    let seg = segment.to_owned();
    let log_dir = log_dir.map(|s| s.to_string());
    let log_level = log_level.map(|s| s.to_lowercase());

    let (tx, rx) = mpsc::channel::<String>();
    LOG_TX.set(tx).expect("LOG_TX already set");

    // Background writer thread with daily rotation and custom dir.
    thread::spawn(move || {
        writer_loop_custom(&app, &seg, rx, log_dir);
    });

    // Register as the global `log` crate backend.
    let max_level = match log_level.as_deref() {
        Some("trace") => LevelFilter::Trace,
        Some("debug") => LevelFilter::Debug,
        Some("info") => LevelFilter::Info,
        Some("warn") => LevelFilter::Warn,
        Some("error") => LevelFilter::Error,
        _ => if debug { LevelFilter::Debug } else { LevelFilter::Warn },
    };

    log::set_logger(&LOGGER)
        .map(|()| log::set_max_level(max_level))
        .expect("Failed to set logger");
}

// Backward compatibility
pub fn init(app_name: &str, segment: &str, debug: bool) {
    init_with_path(app_name, segment, debug, None, None);
}

/// Like writer_loop, but supports a custom log directory and ~ for EXE dir.
fn writer_loop_custom(app_name: &str, segment: &str, rx: mpsc::Receiver<String>, log_dir: Option<String>) {
    let dir = match log_dir {
        Some(mut d) => {
            if d.starts_with("~") {
                if let Ok(exe) = std::env::current_exe() {
                    if let Some(exe_dir) = exe.parent() {
                        d = exe_dir.join(&d[1..]).to_string_lossy().to_string();
                    }
                }
            }
            PathBuf::from(d)
        },
        None => logs_dir(app_name, segment),
    };
    if let Err(e) = fs::create_dir_all(&dir) {
        eprintln!("[Prism][Logger] Failed to create log directory {}: {e}", dir.display());
    } else {
        eprintln!("[Prism][Logger] Log directory: {}", dir.display());
    }

    let mut current_date = today();
    let mut file = open_log_file_custom(&dir, app_name, &current_date);

    if file.is_none() {
        eprintln!("[Prism][Logger] Failed to open log file in {}", dir.display());
    }

    while let Ok(line) = rx.recv() {
        let now_date = today();
        if now_date != current_date {
            current_date = now_date;
            file = open_log_file_custom(&dir, app_name, &current_date);
        }
        if let Some(ref mut f) = file {
            let _ = writeln!(f, "{line}");
            let _ = f.flush();
        }
    }
}

fn open_log_file_custom(
    dir: &PathBuf,
    app_name: &str,
    date: &str,
) -> Option<fs::File> {
    let path = dir.join(format!("{app_name}-{date}.log"));
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok()
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
    // Mirror to stderr for terminal-attached processes (e.g. `cargo run`).
    if stderr_is_tty() {
        eprintln!("[{level}] {msg}");
    }
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

        // Stderr mirroring is handled inside `enqueue` (TTY detection).
        // Keep an extra mirror only when debug is on AND stderr is *not* a TTY
        // (so debugger consoles, attached IDE consoles, etc. still see output).
        if DEBUG_ENABLED.load(Ordering::Relaxed) && !stderr_is_tty() {
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
fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}
