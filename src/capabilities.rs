// OpenRender-runtime/src/capabilities.rs
//
// Runtime capability declarations for OpenRender applications.
//
// Apps import specific capabilities to declare what system resources they need.
// This enables the runtime to conditionally show relevant DevTools panels
// (e.g., Network tab only appears when NetworkAccess is declared) and enforce
// a permission model.
//
// Usage in consuming crates:
//   use OpenRender_runtime::capabilities::{NetworkAccess, StorageAccess};

/// Marker trait for all OpenRender runtime capabilities.
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

    /// Get all declared capability names.
    pub fn names(&self) -> Vec<&'static str> {
        self.capabilities.iter().map(|c| c.name()).collect()
    }
}

impl Default for CapabilitySet {
    fn default() -> Self {
        Self::new()
    }
}
