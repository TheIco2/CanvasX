// prism-runtime/src/env.rs
//
// PRISM environments / surfaces.
//
// PRISM is the rendering layer; **the host application creates the OS-level
// window** (a normal winit window for `Application`, a WorkerW-attached
// surface for `Desktop`, an always-on-top transparent overlay for `Overlay`,
// or a child HWND for `Widget`). PRISM picks sensible defaults for each
// environment via [`Environment::default_capabilities`] and exposes the env
// to scripts so they can adapt their UI.
//
// See `/memories/repo/prism-environments-plan.md` for the full design notes.

use crate::capabilities::{
    CapabilitySet, NetworkAccess, TrayAccess, SingleInstance, Logging, Theming,
};

/// The kind of surface a PRISM scene is rendering into.
///
/// Hosts pick the environment when constructing an [`crate::scene::app_host::AppHost`]
/// via [`crate::scene::app_host::AppHost::with_environment`]. PRISM uses the
/// environment to pick capability defaults and expose a `prism.env` value to
/// JavaScript so pages can adapt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Environment {
    /// Normal application window (Core-UI, ORTestApp, settings panels).
    /// Full capability set, single-instance by default.
    Application,
    /// Wallpaper-style: WorkerW-attached or full-screen-behind-icons.
    /// No tray, no input focus, no clipboard write, multi-instance allowed.
    Desktop,
    /// Always-on-top transparent surface (HUD, MediaFlyout, Game-Bar).
    /// Network ok for live data, no tray, no theming chrome.
    Overlay,
    /// Child HWND embedded inside another host (StatusBar widgets, panels).
    /// Heavily sandboxed: no tray, no single-instance, no logging owner.
    Widget,
}

impl Environment {
    /// Lower-case stable identifier (`"application"`, `"desktop"`, ...).
    /// Exposed to JavaScript as `prism.env`.
    pub fn as_str(self) -> &'static str {
        match self {
            Environment::Application => "application",
            Environment::Desktop     => "desktop",
            Environment::Overlay     => "overlay",
            Environment::Widget      => "widget",
        }
    }

    /// True if the surface participates in normal keyboard focus / IME flow.
    /// Desktop and Widget are *render-only* by default.
    pub fn accepts_focus(self) -> bool {
        matches!(self, Environment::Application | Environment::Overlay)
    }

    /// True if the host should normally allow a system tray icon.
    pub fn allows_tray(self) -> bool {
        matches!(self, Environment::Application)
    }

    /// True if the env should default to single-instance.
    pub fn single_instance_default(self) -> bool {
        matches!(self, Environment::Application)
    }

    /// Build the default [`CapabilitySet`] for this environment. Hosts may
    /// override with [`crate::scene::app_host::AppHost::set_capabilities`]
    /// after construction.
    pub fn default_capabilities(self) -> CapabilitySet {
        let mut caps = CapabilitySet::new();
        caps = caps.declare(Logging);
        match self {
            Environment::Application => {
                caps = caps.declare(NetworkAccess)
                           .declare(TrayAccess)
                           .declare(SingleInstance)
                           .declare(Theming);
            }
            Environment::Desktop => {
                // No tray, no single-instance (per-monitor wallpapers run N copies).
                caps = caps.declare(Theming);
            }
            Environment::Overlay => {
                caps = caps.declare(NetworkAccess);
            }
            Environment::Widget => {
                // Sandboxed: nothing extra. Widget hosts inject what they need.
            }
        }
        caps
    }
}

impl Default for Environment {
    fn default() -> Self { Environment::Application }
}
