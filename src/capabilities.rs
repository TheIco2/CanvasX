// prism-runtime/src/capabilities.rs
//
// Runtime capability declarations for OpenRender applications.
//
// Apps import specific capabilities to declare what system resources they need.
// This enables the runtime to conditionally show relevant DevTools panels
// (e.g., Network tab only appears when NetworkAccess is declared) and enforce
// a permission model.
//
// Usage in consuming crates:
//   use prism_runtime::capabilities::{NetworkAccess, StorageAccess};

/// Marker trait for all Prism runtime capabilities.
pub trait Capability: Send + Sync + 'static {
    /// Human-readable name of this capability.
    fn name(&self) -> &'static str;
    /// Description of what this capability grants.
    fn description(&self) -> &'static str;
}

/// Network/Internet access capability.
/// When declared, the DevTools Network tab becomes visible.
pub struct NetworkAccess;
impl Capability for NetworkAccess {
    fn name(&self) -> &'static str { "Network" }
    fn description(&self) -> &'static str { "Allows the application to make HTTP requests and open network connections." }
}

/// Local storage access capability.
pub struct StorageAccess;
impl Capability for StorageAccess {
    fn name(&self) -> &'static str { "Storage" }
    fn description(&self) -> &'static str { "Allows the application to read and write local persistent storage." }
}

/// IPC communication capability (already implicit for OpenDesktop apps).
pub struct IpcAccess;
impl Capability for IpcAccess {
    fn name(&self) -> &'static str { "IPC" }
    fn description(&self) -> &'static str { "Allows inter-process communication with the host application." }
}

/// System information access (CPU, memory, GPU sensors, etc.).
pub struct SystemInfo;
impl Capability for SystemInfo {
    fn name(&self) -> &'static str { "SystemInfo" }
    fn description(&self) -> &'static str { "Allows reading system hardware and performance information." }
}

/// File system access capability.
pub struct FileSystemAccess;
impl Capability for FileSystemAccess {
    fn name(&self) -> &'static str { "FileSystem" }
    fn description(&self) -> &'static str { "Allows reading and writing files on the local file system." }
}

/// Peripheral/device access (USB HID, serial ports, etc.).
pub struct DeviceAccess;
impl Capability for DeviceAccess {
    fn name(&self) -> &'static str { "Devices" }
    fn description(&self) -> &'static str { "Allows communication with connected peripherals and devices." }
}

/// System tray access capability.
/// When declared, the application can show a system tray icon
/// with configurable menu items and minimize-to-tray behaviour.
pub struct TrayAccess;
impl Capability for TrayAccess {
    fn name(&self) -> &'static str { "Tray" }
    fn description(&self) -> &'static str { "Allows the application to display a system tray icon and menu." }
}

/// Logging capability.
///
/// When declared (or implicitly enabled), the runtime initializes the file
/// + stderr logger using the loaded `LoggingConfig`. Disabling this skips
/// logger setup entirely (only `eprintln!` diagnostics will be visible).
pub struct Logging;
impl Capability for Logging {
    fn name(&self) -> &'static str { "Logging" }
    fn description(&self) -> &'static str { "Initializes the runtime logger and writes log files to disk." }
}

/// Theming capability.
///
/// When declared (or implicitly enabled), the runtime loads theme JSON
/// files from the embedded `pages/themes/` directory and from the
/// `<install>/themes/` directory next to the EXE. The active theme is
/// selected via `config.default.json` (key `theme`).
pub struct Theming;
impl Capability for Theming {
    fn name(&self) -> &'static str { "Theming" }
    fn description(&self) -> &'static str { "Loads user-selectable theme files and injects their CSS into every page." }
}

/// Single-instance capability.
///
/// When declared, only one instance of the application can run at a time.
/// Launching the EXE while already running will bring the existing window
/// into focus (showing it first if hidden/minimized to tray).
///
/// Enforcement uses a named mutex and named pipe on Windows.
pub struct SingleInstance;
impl Capability for SingleInstance {
    fn name(&self) -> &'static str { "SingleInstance" }
    fn description(&self) -> &'static str { "Restricts the application to a single running instance. Re-launching focuses the existing window." }
}

/// Multi-instance capability.
///
/// When declared, the application explicitly supports running multiple
/// instances simultaneously. All instances share a single system tray
/// icon, and built-in tray actions (Exit, Reload) apply to every instance.
///
/// Custom tray actions can be routed to a specific subset of instances
/// via `MultiInstanceRouting`.
pub struct MultiInstance;
impl Capability for MultiInstance {
    fn name(&self) -> &'static str { "MultiInstance" }
    fn description(&self) -> &'static str { "Allows multiple instances sharing a single system tray with configurable action routing." }
}

/// Routing strategy for custom tray actions in multi-instance mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MultiInstanceRouting {
    /// Custom actions are sent to all running instances.
    All,
    /// Custom actions are sent only to the instance that was last focused.
    LastFocused,
    /// Custom actions are sent only to the first instance that was launched.
    FirstLaunched,
}

/// A set of declared capabilities for a OpenRender application.
pub struct CapabilitySet {
    capabilities: Vec<Box<dyn Capability>>,
}

impl CapabilitySet {
    /// Create an empty capability set.
    pub fn new() -> Self {
        Self { capabilities: Vec::new() }
    }

    /// Declare a capability.
    pub fn declare<C: Capability>(mut self, cap: C) -> Self {
        self.capabilities.push(Box::new(cap));
        self
    }

    /// Check if a capability of a given type is declared.
    pub fn has<C: Capability + 'static>(&self) -> bool {
        self.capabilities.iter().any(|c| {
            // Use type name comparison since we can't downcast trait objects easily
            std::any::type_name::<C>().ends_with(c.name())
                || c.name() == std::any::type_name::<C>().rsplit("::").next().unwrap_or("")
        })
    }

    /// Check if network access is declared (convenience method).
    pub fn has_network(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "Network")
    }

    /// Check if tray access is declared (convenience method).
    pub fn has_tray(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "Tray")
    }

    /// Check if single-instance mode is declared (convenience method).
    pub fn has_single_instance(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "SingleInstance")
    }

    /// Check if multi-instance mode is declared (convenience method).
    pub fn has_multi_instance(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "MultiInstance")
    }

    /// Check if logging is enabled.
    pub fn has_logging(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "Logging")
    }

    /// Check if theming is enabled.
    pub fn has_theming(&self) -> bool {
        self.capabilities.iter().any(|c| c.name() == "Theming")
    }

    /// Get all declared capability names.
    pub fn names(&self) -> Vec<&'static str> {
        self.capabilities.iter().map(|c| c.name()).collect()
    }

    /// Check whether the set contains a capability with the given name
    /// (case-insensitive). Use this for config-file-driven gating.
    pub fn contains(&self, name: &str) -> bool {
        self.capabilities
            .iter()
            .any(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// Declare a capability by its string name. Unknown names are ignored
    /// (a warning is logged). This is the entry point used by config-file
    /// loading; see [`KNOWN_CAPABILITIES`] for the list of recognised names.
    pub fn declare_by_name(mut self, name: &str) -> Self {
        if let Some(cap) = capability_from_name(name) {
            self.capabilities.push(cap);
        } else {
            log::warn!(
                "[prism::capabilities] unknown capability '{name}' - ignored. Known: {:?}",
                KNOWN_CAPABILITIES
            );
        }
        self
    }

    /// Declare every capability listed in `names`, ignoring unknowns.
    pub fn declare_all<I, S>(mut self, names: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        for n in names {
            self = self.declare_by_name(n.as_ref());
        }
        self
    }
}

// ---------------------------------------------------------------------------
// String-based registry
// ---------------------------------------------------------------------------

/// All capability names recognised by [`capability_from_name`] and
/// [`CapabilitySet::declare_by_name`]. To add a new capability, define a
/// new struct + `Capability` impl above and add a match arm in
/// [`capability_from_name`] plus an entry here.
pub const KNOWN_CAPABILITIES: &[&str] = &[
    "Network",
    "Storage",
    "IPC",
    "SystemInfo",
    "FileSystem",
    "Devices",
    "Tray",
    "SingleInstance",
    "MultiInstance",
    "Logging",
    "Theming",
];

/// Capabilities that are enabled by default unless explicitly disabled in
/// `config.prism.json` via the `"!name"` (negation) prefix.
pub const IMPLICIT_CAPABILITIES: &[&str] = &[
    "Logging",
    "Theming",
    "Storage",
    "IPC",
];

/// Map a config-file capability name to a boxed [`Capability`] instance.
/// Names are matched case-insensitively. Returns `None` for unknown names.
pub fn capability_from_name(name: &str) -> Option<Box<dyn Capability>> {
    match name.to_ascii_lowercase().as_str() {
        "network" | "networkaccess" => Some(Box::new(NetworkAccess)),
        "storage" | "storageaccess" => Some(Box::new(StorageAccess)),
        "ipc" | "ipcaccess" => Some(Box::new(IpcAccess)),
        "systeminfo" | "system" => Some(Box::new(SystemInfo)),
        "filesystem" | "fs" | "filesystemaccess" => Some(Box::new(FileSystemAccess)),
        "devices" | "device" | "deviceaccess" => Some(Box::new(DeviceAccess)),
        "tray" | "trayaccess" => Some(Box::new(TrayAccess)),
        "singleinstance" | "single" => Some(Box::new(SingleInstance)),
        "multiinstance" | "multi" => Some(Box::new(MultiInstance)),
        "logging" | "log" => Some(Box::new(Logging)),
        "theming" | "theme" | "themes" => Some(Box::new(Theming)),
        _ => None,
    }
}

/// Build a [`CapabilitySet`] from a list of config-file capability strings,
/// honouring negation (`"!name"`) and implicit (default-on) capabilities.
///
/// Algorithm:
///   1. Start with [`IMPLICIT_CAPABILITIES`].
///   2. For each entry in `entries`:
///       - if it begins with `!`, mark the capability as disabled.
///       - otherwise, mark it as enabled.
///   3. Final set = (implicit ∪ enabled) − disabled.
pub fn resolve_from_config<I, S>(entries: I) -> CapabilitySet
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    use std::collections::BTreeSet;

    let mut enabled: BTreeSet<String> = IMPLICIT_CAPABILITIES
        .iter()
        .map(|s| s.to_ascii_lowercase())
        .collect();
    let mut disabled: BTreeSet<String> = BTreeSet::new();

    for raw in entries {
        let s = raw.as_ref().trim();
        if s.is_empty() { continue; }
        if let Some(rest) = s.strip_prefix('!') {
            disabled.insert(rest.trim().to_ascii_lowercase());
        } else {
            enabled.insert(s.to_ascii_lowercase());
        }
    }

    let mut set = CapabilitySet::new();
    for name in enabled.difference(&disabled) {
        if let Some(cap) = capability_from_name(name) {
            set.capabilities.push(cap);
        } else {
            log::warn!(
                "[prism::capabilities] unknown capability '{name}' in config (ignored). Known: {:?}",
                KNOWN_CAPABILITIES
            );
        }
    }
    set
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self::new()
    }
}

