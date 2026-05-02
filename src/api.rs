// prism-runtime/src/api.rs
//
// High-level convenience API. Lets a consumer write:
//
//     use include_dir::{include_dir, Dir};
//
//     static PAGES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/pages");
//
//     fn main() {
//         prism_runtime::run(prism_runtime::EmbeddedApp {
//             prism_config_json:   include_str!("../config.prism.json"),
//             default_config_json: Some(include_str!("../config.default.json")),
//             pages:               &PAGES,
//         }).unwrap();
//     }
//
// Behind the scenes:
//
//   1. The embedded app bundle (config + `pages/`) is installed.
//   2. `config.prism.json` is parsed from the embedded bytes.
//   3. `config.default.json` from `<exe-dir>` (or CWD) is loaded and merged.
//   4. The capability set is resolved (implicit + developer + user, with
//      `!negation`).
//   5. Logging is initialised if the `Logging` capability is active.
//   6. Themes are loaded if the `Theming` capability is active and the
//      configured theme is selected.
//   7. The AppHost is built (landing page + sidebar pages, all from the
//      embedded `pages/` directory) and the window opens.

use std::sync::Mutex;

use crate::capabilities::{self, CapabilitySet};
use crate::config::{ConfigError, DefaultConfig, PrismConfig};
use crate::embed::{self, EmbeddedApp};
use crate::logging;
use crate::run::{self, WindowOptions};
use crate::scene::app_host::{AppHost, PageSource, Route};
use crate::theming::ThemeRegistry;

/// Process-wide singleton state.
pub struct Prism {
    inner: Mutex<Option<PrismConfig>>,
    logger_initialized: Mutex<bool>,
}

pub static PRISM: Prism = Prism::new();

#[derive(Debug)]
pub enum StartError {
    NoConfig,
    Config(ConfigError),
    Run(String),
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StartError::NoConfig => write!(f, "prism: no config loaded"),
            StartError::Config(e) => write!(f, "{e}"),
            StartError::Run(e) => write!(f, "runtime error: {e}"),
        }
    }
}

impl std::error::Error for StartError {}

impl From<ConfigError> for StartError {
    fn from(e: ConfigError) -> Self { StartError::Config(e) }
}

impl Prism {
    pub const fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            logger_initialized: Mutex::new(false),
        }
    }

    pub fn current_config(&self) -> Option<PrismConfig> {
        self.inner.lock().expect("PRISM mutex poisoned").clone()
    }

    fn store_config(&self, cfg: &PrismConfig) {
        *self.inner.lock().expect("PRISM mutex poisoned") = Some(cfg.clone());
    }

    fn maybe_init_logger(&self, cfg: &PrismConfig, caps: &CapabilitySet) {
        if !caps.has_logging() {
            eprintln!("[Prism] Logging capability disabled — skipping logger init");
            return;
        }
        let mut guard = self.logger_initialized.lock().unwrap();
        if *guard { return; }

        let app_name = cfg.title.clone().unwrap_or_else(|| "Prism".to_string());
        let log_cfg = cfg.logging.clone().unwrap_or_default();
        let log_dir = log_cfg.file.as_deref();
        let log_level = Some(log_cfg.level.as_str());
        let verbose = log_level == Some("debug") || log_level == Some("trace");
        eprintln!(
            "[Prism] Initializing logger: app_name={}, dir={:?}, level={:?}",
            app_name, log_dir, log_level
        );
        logging::init_with_path(&app_name, "Prism", verbose, log_dir, log_level);
        *guard = true;
    }
}

impl Default for Prism {
    fn default() -> Self { Self::new() }
}

// ---------------------------------------------------------------------------
// Public free functions
// ---------------------------------------------------------------------------

/// One-call entry point. Installs the embedded app bundle, loads + merges
/// configs, builds the host, and runs the window. Blocks until the window
/// closes.
pub fn run(app: EmbeddedApp) -> Result<(), StartError> {
    embed::install(app);
    seed_user_files();
    start()
}

/// Start the runtime using the already-installed embedded app. Useful when
/// the app needs to perform additional setup between embedding and starting.
pub fn start() -> Result<(), StartError> {
    let app = embed::get().ok_or(StartError::Config(ConfigError::NotEmbedded))?;

    // 1. Parse the developer-locked config from the embedded JSON.
    let mut cfg = PrismConfig::from_json(app.prism_config_json)?;

    // 2. Merge runtime overrides from <exe-dir>/config.default.json (or CWD).
    let overrides = DefaultConfig::load().unwrap_or_default();
    cfg.apply_overrides(&overrides);

    // 3. Resolve capabilities: implicit ∪ developer ∪ user, minus !negations.
    let caps = capabilities::resolve_from_config(cfg.capabilities.iter().cloned());
    eprintln!("[Prism] Active capabilities: {:?}", caps.names());

    // 4. Logger.
    PRISM.maybe_init_logger(&cfg, &caps);
    log::info!("[Prism] Active capabilities: {:?}", caps.names());

    // 5. Stash config for `current_config()` consumers.
    PRISM.store_config(&cfg);

    // 6. Themes.
    let theme_css = if caps.has_theming() {
        let mut reg = ThemeRegistry::load_all();
        let want = cfg.theme.as_deref().unwrap_or("");
        if !want.is_empty() && reg.set_active(want) {
            log::info!("[Prism] Active theme: {}", want);
            Some(reg.active_css())
        } else if !reg.names().is_empty() {
            // Fall back to the first available theme.
            let first = reg.names()[0].to_string();
            reg.set_active(&first);
            log::info!("[Prism] Active theme (fallback): {}", first);
            Some(reg.active_css())
        } else {
            None
        }
    } else {
        None
    };

    // 7. Build the host.
    let host = build_host(&cfg, caps, theme_css);

    let opts = WindowOptions {
        title: cfg.title.clone().unwrap_or_else(|| "Prism".into()),
        width: cfg.window.as_ref().map(|w| w.width).unwrap_or(1024),
        height: cfg.window.as_ref().map(|w| w.height).unwrap_or(768),
    };

    run::run_app(host, opts).map_err(|e| StartError::Run(e.to_string()))
}

/// Configure the runtime with an already-loaded `PrismConfig` (advanced).
/// The embedded app must already be installed via [`run`] or directly via
/// [`embed::install`].
pub fn start_with(cfg: PrismConfig) -> Result<(), StartError> {
    if embed::get().is_none() {
        return Err(StartError::Config(ConfigError::NotEmbedded));
    }
    let caps = capabilities::resolve_from_config(cfg.capabilities.iter().cloned());
    PRISM.maybe_init_logger(&cfg, &caps);
    PRISM.store_config(&cfg);
    let theme_css = if caps.has_theming() {
        let mut reg = ThemeRegistry::load_all();
        if let Some(t) = cfg.theme.as_deref() { reg.set_active(t); }
        Some(reg.active_css())
    } else { None };
    let host = build_host(&cfg, caps, theme_css);
    let opts = WindowOptions {
        title: cfg.title.clone().unwrap_or_else(|| "Prism".into()),
        width: cfg.window.as_ref().map(|w| w.width).unwrap_or(1024),
        height: cfg.window.as_ref().map(|w| w.height).unwrap_or(768),
    };
    run::run_app(host, opts).map_err(|e| StartError::Run(e.to_string()))
}

// ---------------------------------------------------------------------------
// Internal: turn a PrismConfig into a populated AppHost using embedded pages.
// ---------------------------------------------------------------------------

fn build_host(cfg: &PrismConfig, caps: CapabilitySet, theme_css: Option<String>) -> AppHost {
    let title = cfg.title.clone().unwrap_or_else(|| "Prism".into());
    let mut host = AppHost::new(&title);
    host.set_capabilities(caps);
    host.set_theme_css(theme_css);

    // Every embedded *.html in `pages/` (excluding components/, themes/) becomes
    // a route keyed by its file stem.
    for f in embed::pages_matching(|p| {
        p.ends_with(".html")
            && !p.starts_with("components/")
            && !p.starts_with("themes/")
    }) {
        let rel = f.path().to_string_lossy().replace('\\', "/");
        let stem = std::path::Path::new(&rel)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("page")
            .to_string();
        host.add_route(make_route(&stem, &rel));
    }

    // Landing page — defaults to "base.html" if not specified.
    let landing_file = cfg
        .pages
        .landing
        .clone()
        .unwrap_or_else(|| "base.html".to_string());
    let landing_id = std::path::Path::new(&landing_file)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("base")
        .to_string();

    if !host.routes().iter().any(|r| r.id == landing_id) {
        host.add_route(make_route(&landing_id, &landing_file));
    }

    // Tell the host which route to treat as "Home" (used by the right-click
    // Home action and the configurable context-menu).
    host.set_home_route(&landing_id);

    // Configure the right-click context menu from the developer config.
    if let Some(cm) = cfg.context_menu.as_ref() {
        host.configure_context_menu(&cm.items, &cm.hide_defaults);
    }

    host.navigate_to(&landing_id);

    host
}

fn make_route(id: &str, embedded_rel: &str) -> Route {
    Route {
        id: id.to_string(),
        label: id.to_string(),
        icon: None,
        source: PageSource::Embedded(embedded_rel.to_string()),
        separator: false,
    }
}

// ---------------------------------------------------------------------------
// First-run user file seeding
// ---------------------------------------------------------------------------

/// On startup, ensure the install directory has a writable copy of every
/// user-editable file:
///
///   * `<exe-dir>/config.default.json`  — from `EmbeddedApp::default_config_json`
///   * `<exe-dir>/themes/*.json`        — copied from embedded `pages/themes/`
///
/// Existing files are left untouched (the user's customisations win).
fn seed_user_files() {
    let Some(app) = embed::get() else { return; };
    let Ok(exe) = std::env::current_exe() else { return; };
    let Some(dir) = exe.parent() else { return; };

    // 1. Seed config.default.json if absent.
    if let Some(text) = app.default_config_json {
        let path = dir.join("config.default.json");
        if !path.exists() {
            if let Err(e) = std::fs::write(&path, text) {
                log::warn!("[Prism] could not seed {}: {e}", path.display());
            } else {
                log::info!("[Prism] seeded {}", path.display());
            }
        }
    }

    // 2. Seed themes/ from embedded pages/themes/*.json.
    let themes_dir = dir.join("themes");
    let _ = std::fs::create_dir_all(&themes_dir);
    for f in embed::pages_matching(|p| p.starts_with("themes/") && p.ends_with(".json")) {
        let Some(name) = std::path::Path::new(&f.path().to_string_lossy().to_string())
            .file_name()
            .map(|n| n.to_owned())
        else { continue; };
        let dest = themes_dir.join(name);
        if dest.exists() { continue; }
        if let Err(e) = std::fs::write(&dest, f.contents()) {
            log::warn!("[Prism] could not seed theme {}: {e}", dest.display());
        }
    }
}
