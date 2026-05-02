// prism-runtime/src/theming.rs
//
// Theme system for Prism applications.
//
// A theme is a JSON document of the form:
//
// ```json
// {
//   "name": "Dark",
//   "variables": {
//     "--bg":   "#111317",
//     "--fg":   "#e9eaee",
//     "--accent": "#7ab8ff"
//   },
//   "css": "body { backdrop-filter: blur(10px); }"
// }
// ```
//
// Themes are loaded from two locations and merged (user wins on conflict):
//
//   * Embedded `pages/themes/*.json` (developer-shipped defaults).
//   * `<install>/themes/*.json` (user-added/customised).
//
// The active theme name comes from `config.default.json` (`theme`), with a
// fall-back to `PrismConfig::theme`. The merged CSS is injected into every
// page that the AppHost compiles.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::embed;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Theme {
    /// Display name of the theme. If absent, falls back to the file stem.
    #[serde(default)]
    pub name: Option<String>,

    /// CSS custom-property overrides applied to `:root` of every page.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,

    /// Free-form CSS appended after the variable block.
    #[serde(default)]
    pub css: Option<String>,
}

impl Theme {
    /// Compile the theme into a CSS snippet ready to be injected into a page.
    pub fn to_css(&self) -> String {
        let mut out = String::new();
        if !self.variables.is_empty() {
            out.push_str(":root {\n");
            for (k, v) in &self.variables {
                out.push_str("    ");
                out.push_str(k);
                out.push_str(": ");
                out.push_str(v);
                out.push_str(";\n");
            }
            out.push_str("}\n");
        }
        if let Some(ref css) = self.css {
            out.push_str(css);
            out.push('\n');
        }
        out
    }
}

/// In-memory registry of all themes available to the running app.
#[derive(Debug, Default)]
pub struct ThemeRegistry {
    themes: BTreeMap<String, Theme>,
    active: Option<String>,
}

impl ThemeRegistry {
    pub fn new() -> Self { Self::default() }

    /// Load themes from the embedded `pages/themes/*.json` directory and from
    /// `<exe-dir>/themes/*.json` (user-supplied overrides).
    pub fn load_all() -> Self {
        let mut reg = Self::new();
        reg.load_embedded();
        reg.load_user();
        reg
    }

    /// Load every embedded `pages/themes/*.json` into the registry.
    pub fn load_embedded(&mut self) {
        for f in embed::pages_matching(|p| p.starts_with("themes/") && p.ends_with(".json")) {
            let bytes = f.contents();
            let text = match std::str::from_utf8(bytes) {
                Ok(t) => t,
                Err(_) => continue,
            };
            if text.trim().is_empty() {
                continue;
            }
            let mut theme: Theme = match serde_json::from_str(text) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("[prism::theming] failed to parse {:?}: {e}", f.path());
                    continue;
                }
            };
            let name = theme.name.clone().unwrap_or_else(|| {
                f.path()
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("theme")
                    .to_string()
            });
            theme.name = Some(name.clone());
            self.themes.insert(name, theme);
        }
    }

    /// Load every `<exe-dir>/themes/*.json` into the registry. User themes
    /// override embedded themes of the same name.
    pub fn load_user(&mut self) {
        let Some(dir) = user_themes_dir() else { return; };
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => return,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let text = match std::fs::read_to_string(&path) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("[prism::theming] read {}: {e}", path.display());
                    continue;
                }
            };
            if text.trim().is_empty() {
                continue;
            }
            let mut theme: Theme = match serde_json::from_str(&text) {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("[prism::theming] parse {}: {e}", path.display());
                    continue;
                }
            };
            let name = theme.name.clone().unwrap_or_else(|| {
                path.file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("theme")
                    .to_string()
            });
            theme.name = Some(name.clone());
            self.themes.insert(name, theme);
        }
    }

    /// Set the active theme by name (case-insensitive). Returns `true` if a
    /// matching theme exists.
    pub fn set_active(&mut self, name: &str) -> bool {
        let key = self
            .themes
            .keys()
            .find(|k| k.eq_ignore_ascii_case(name))
            .cloned();
        match key {
            Some(k) => {
                self.active = Some(k);
                true
            }
            None => false,
        }
    }

    /// Return the active theme, if any.
    pub fn active(&self) -> Option<&Theme> {
        self.active.as_ref().and_then(|n| self.themes.get(n))
    }

    /// Compile the active theme into a CSS snippet (empty string if none).
    pub fn active_css(&self) -> String {
        self.active().map(|t| t.to_css()).unwrap_or_default()
    }

    /// All known theme names, in alphabetical order.
    pub fn names(&self) -> Vec<&str> {
        self.themes.keys().map(|s| s.as_str()).collect()
    }
}

/// Resolve `<exe-dir>/themes/`, creating the directory if it doesn't exist
/// (best-effort; failures are ignored).
fn user_themes_dir() -> Option<std::path::PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?.join("themes");
    let _ = std::fs::create_dir_all(&dir);
    Some(dir)
}

/// Convenience: `<install>/themes/` path (read-only access used by the
/// installer to seed user-editable themes from embedded ones).
pub fn user_themes_path(install_dir: &Path) -> std::path::PathBuf {
    install_dir.join("themes")
}
