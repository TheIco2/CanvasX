// canvasx-runtime/src/capabilities.rs
//
// Runtime capability declarations for CanvasX applications.
//
// Apps import specific capabilities to declare what system resources they need.
// This enables the runtime to conditionally show relevant DevTools panels
// (e.g., Network tab only appears when NetworkAccess is declared) and enforce
// a permission model.
//
// Usage in consuming crates:
//   use canvasx_runtime::capabilities::{NetworkAccess, StorageAccess};

/// Marker trait for all CanvasX runtime capabilities.
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

/// IPC communication capability (already implicit for Sentinel apps).
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

/// A set of declared capabilities for a CanvasX application.
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
