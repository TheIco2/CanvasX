// prism-runtime/src/embed.rs
//
// Compile-time-embedded application bundle: the developer-locked
// `config.prism.json` and the entire `pages/` directory.
//
// The consuming crate (the application) calls the [`prism_app!`] macro
// inside `main()`, which expands to an [`EmbeddedApp`] value (built via
// `include_str!` + `include_dir::include_dir!`). The value is then passed to
// [`crate::run`] which installs it into the process-wide registry.

use std::sync::OnceLock;

#[doc(hidden)]
pub use include_dir::{Dir, DirEntry, File};

/// Snapshot of the developer-locked, compiled-in app assets.
#[derive(Clone, Copy)]
pub struct EmbeddedApp {
    /// Raw text of `config.prism.json`.
    pub prism_config_json: &'static str,
    /// Optional raw text of a developer-shipped default `config.default.json`
    /// (used as a template if the user's install dir does not contain one).
    pub default_config_json: Option<&'static str>,
    /// Embedded `pages/` tree. All HTML/CSS/JS/JSON live here.
    pub pages: &'static Dir<'static>,
}

/// The single, process-wide app bundle. Set by [`install`].
static EMBED: OnceLock<EmbeddedApp> = OnceLock::new();

/// Install the app bundle into the process-wide registry. Subsequent calls
/// are no-ops (the first registration wins).
pub fn install(app: EmbeddedApp) {
    let _ = EMBED.set(app);
}

/// Get the embedded app bundle, if it has been installed.
pub fn get() -> Option<EmbeddedApp> {
    EMBED.get().copied()
}

/// Lookup an embedded file under `pages/` by its relative path
/// (e.g. `"base.html"`, `"themes/Dark.json"`).
pub fn read_page_bytes(rel_path: &str) -> Option<&'static [u8]> {
    let app = EMBED.get()?;
    let f = app.pages.get_file(rel_path)?;
    Some(f.contents())
}

/// Lookup an embedded UTF-8 file under `pages/`.
pub fn read_page_str(rel_path: &str) -> Option<&'static str> {
    let bytes = read_page_bytes(rel_path)?;
    std::str::from_utf8(bytes).ok()
}

/// Iterate every file under `pages/` whose path matches a predicate.
pub fn pages_matching<F>(predicate: F) -> Vec<&'static File<'static>>
where
    F: Fn(&str) -> bool,
{
    let Some(app) = EMBED.get() else { return Vec::new(); };
    let mut out = Vec::new();
    collect(app.pages, &predicate, &mut out);
    out
}

fn collect<F>(dir: &'static Dir<'static>, pred: &F, out: &mut Vec<&'static File<'static>>)
where
    F: Fn(&str) -> bool,
{
    for entry in dir.entries() {
        match entry {
            DirEntry::File(f) => {
                if let Some(path) = f.path().to_str() {
                    if pred(path) {
                        out.push(f);
                    }
                }
            }
            DirEntry::Dir(d) => collect(d, pred, out),
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------
//
// To embed your app, in the consumer crate's `main.rs`:
//
// ```ignore
// use include_dir::{include_dir, Dir};
//
// static PAGES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/pages");
//
// fn main() {
//     prism_runtime::run(prism_runtime::EmbeddedApp {
//         prism_config_json: include_str!("../config.prism.json"),
//         default_config_json: Some(include_str!("../config.default.json")),
//         pages: &PAGES,
//     }).unwrap();
// }
// ```

