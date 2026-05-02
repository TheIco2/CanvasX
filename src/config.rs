// prism-runtime/src/config.rs
//
// Declarative configuration for Prism applications.
//
// Two configs participate in startup:
//
//   * `config.prism.json`  — developer-locked. Embedded into the EXE at
//                            compile time via `prism_runtime::prism_app!()`.
//                            Defines capabilities, install metadata, default
//                            window size, landing page name, etc.
//
//   * `config.default.json` — user-tweakable settings copied to the install
//                            directory. Loaded at runtime; any field present
//                            here overrides the same field in `prism.json`.
//                            Used for things like the active theme or runtime
//                            log level.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Developer-locked config (embedded in EXE)
// ---------------------------------------------------------------------------

/// Top-level Prism configuration. Embedded into the binary at compile time.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PrismConfig {
    /// Capability names the application requires (e.g. "tray", "network").
    /// Names are matched case-insensitively. Prefix a name with `!` to
    /// disable a capability that would otherwise be enabled implicitly
    /// (see `crate::capabilities::IMPLICIT_CAPABILITIES`).
    #[serde(default)]
    pub capabilities: Vec<String>,

    /// Page configuration (landing page id, etc.).
    #[serde(default)]
    pub pages: PagesConfig,

    /// Window title (defaults to "Prism").
    #[serde(default)]
    pub title: Option<String>,

    /// Initial window size in logical pixels.
    #[serde(default)]
    pub window: Option<WindowConfig>,

    /// Logging configuration.
    #[serde(default)]
    pub logging: Option<LoggingConfig>,

    /// Install configuration (used by the installer).
    #[serde(default)]
    pub install: Option<InstallConfig>,

    /// Default theme to select if the user has not chosen one in
    /// `config.default.json`.
    #[serde(default)]
    pub theme: Option<String>,

    /// Right-click context menu customisation. The default menu (Inspect,
    /// DevTools, Pop-Out DevTools, Debug Server, Home, Back, Forward, Reload,
    /// Exit) is always available; this block lets a developer add custom
    /// items and/or hide individual built-in items by name.
    #[serde(default)]
    pub context_menu: Option<ContextMenuConfig>,
}

/// Configurable extras for the right-click context menu.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ContextMenuConfig {
    /// Custom items to append (after a separator) to the built-in menu.
    #[serde(default)]
    pub items: Vec<ContextMenuItemConfig>,
    /// Names of built-in items to hide. Case-insensitive. Recognised names:
    /// `inspect`, `devtools`, `popout-devtools`, `debug-server`, `home`,
    /// `back`, `forward`, `reload`, `exit`.
    #[serde(default)]
    pub hide_defaults: Vec<String>,
}

/// One developer-defined context-menu entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContextMenuItemConfig {
    /// Visible label. Use `"-"` (or set `separator: true`) to render a divider.
    pub label: String,
    /// Action keyword. Recognised values:
    ///
    ///   * `home`, `back`, `forward`, `reload`
    ///   * `devtools`, `popout-devtools`, `debug-server`, `inspect`
    ///   * `exit`
    ///   * `navigate:<page-id>` — switch to a named route
    ///   * `js:<expression>`    — evaluate JavaScript in the active page
    #[serde(default)]
    pub action: Option<String>,
    /// Optional shortcut hint shown right-aligned (purely cosmetic).
    #[serde(default)]
    pub shortcut: Option<String>,
    /// Render this entry as a separator. Overrides `label`/`action`.
    #[serde(default)]
    pub separator: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct LoggingConfig {
    #[serde(default = "default_log_level")]
    pub level: String,
    /// Optional log file directory. `~` is expanded to the user's home.
    #[serde(default)]
    pub file: Option<String>,
}

fn default_log_level() -> String { "info".to_string() }

/// Page configuration. With embedded pages there is no need to specify on-disk
/// directories; the developer just names which embedded HTML file is the
/// landing page.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PagesConfig {
    /// File name of the landing-page HTML *relative to the embedded `pages/`
    /// root* (e.g. `"base.html"`). Defaults to `"base.html"` if omitted.
    #[serde(default)]
    pub landing: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WindowConfig {
    pub width: u32,
    pub height: u32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct InstallConfig {
    /// Default install location (used by the installer).
    #[serde(default)]
    pub location: Option<String>,
    /// Whether to create a desktop shortcut on install.
    #[serde(default)]
    pub create_shortcut: bool,
    /// Whether to register the app to launch at user login.
    #[serde(default)]
    pub run_at_startup: bool,
}

// ---------------------------------------------------------------------------
// User-editable runtime overrides
// ---------------------------------------------------------------------------

/// Runtime overrides loaded from `config.default.json` next to the EXE (or in
/// the current working directory). All fields are optional; any field that is
/// `Some` (or non-empty) overrides the corresponding field in [`PrismConfig`].
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DefaultConfig {
    #[serde(default)]
    pub theme: Option<String>,

    #[serde(default)]
    pub logging: Option<LoggingConfig>,

    #[serde(default)]
    pub window: Option<WindowConfig>,

    /// User capability overrides. Same syntax as `PrismConfig::capabilities`
    /// (negation with `!` is honoured). Appended to the developer set.
    #[serde(default)]
    pub capabilities: Vec<String>,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum ConfigError {
    Io(std::io::Error),
    Parse(serde_json::Error),
    /// Landing page id was not found in the embedded `pages/` registry.
    LandingMissing(String),
    /// `prism_runtime::prism_app!()` was never invoked, so no embedded config
    /// or pages exist.
    NotEmbedded,
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io(e) => write!(f, "config I/O error: {e}"),
            ConfigError::Parse(e) => write!(f, "config parse error: {e}"),
            ConfigError::LandingMissing(p) => {
                write!(f, "landing page '{p}' not found in embedded pages")
            }
            ConfigError::NotEmbedded => write!(
                f,
                "no embedded app — call `prism_runtime::prism_app!(...)` in your crate root"
            ),
        }
    }
}

impl std::error::Error for ConfigError {}

impl From<std::io::Error> for ConfigError {
    fn from(e: std::io::Error) -> Self { ConfigError::Io(e) }
}

impl From<serde_json::Error> for ConfigError {
    fn from(e: serde_json::Error) -> Self { ConfigError::Parse(e) }
}

// ---------------------------------------------------------------------------
// Loading & merging
// ---------------------------------------------------------------------------

impl PrismConfig {
    /// Parse a developer config from a JSON string (typically the embedded
    /// `config.prism.json`).
    pub fn from_json(text: &str) -> Result<Self, ConfigError> {
        Ok(serde_json::from_str(text)?)
    }

    /// Apply runtime overrides from a [`DefaultConfig`] in place. Only fields
    /// that are `Some` (or non-empty for vectors) on the override are taken.
    pub fn apply_overrides(&mut self, overrides: &DefaultConfig) {
        if overrides.theme.is_some() {
            self.theme = overrides.theme.clone();
        }
        if let Some(ref l) = overrides.logging {
            self.logging = Some(l.clone());
        }
        if let Some(ref w) = overrides.window {
            self.window = Some(w.clone());
        }
        if !overrides.capabilities.is_empty() {
            self.capabilities.extend(overrides.capabilities.iter().cloned());
        }
    }
}

impl DefaultConfig {
    /// Try to load `config.default.json` from the given directory. Returns
    /// `Ok(Default::default())` if the file does not exist.
    pub fn load_from_dir(dir: &std::path::Path) -> Result<Self, ConfigError> {
        let path = dir.join("config.default.json");
        match std::fs::read_to_string(&path) {
            Ok(text) if !text.trim().is_empty() => Ok(serde_json::from_str(&text)?),
            Ok(_) => Ok(Self::default()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(ConfigError::Io(e)),
        }
    }

    /// Try several well-known locations for `config.default.json`:
    ///
    ///   1. The directory containing the running executable.
    ///   2. The current working directory.
    ///
    /// Returns the first one found, or `Default::default()` if none exist.
    pub fn load() -> Result<Self, ConfigError> {
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                let p = dir.join("config.default.json");
                if p.exists() {
                    return Self::load_from_dir(dir);
                }
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            return Self::load_from_dir(&cwd);
        }
        Ok(Self::default())
    }
}

/// Expand a leading `~` in a path string to the user's home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        if let Some(home) = std::env::var_os("USERPROFILE").or_else(|| std::env::var_os("HOME")) {
            let mut p = PathBuf::from(home);
            let trimmed = rest.trim_start_matches(['/', '\\']);
            if !trimmed.is_empty() {
                p.push(trimmed);
            }
            return p;
        }
    }
    PathBuf::from(path)
}
