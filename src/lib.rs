// prism-runtime — GPU-native deterministic UI runtime for OpenRender
//
// Architecture:
//   HTML/CSS/JS → Compiler → ORD (Prism Runtime Document) → GPU Renderer
//
// Supports: Vulkan, DirectX 12, DirectX 11 (via wgpu backends)
//
// This crate is the rendering engine binary. host application talks to it via
// IPC (named pipes) to push system data and control scenes (wallpapers,
// widgets, status bar).

pub mod logging;
pub mod prd;

pub mod gpu;
pub mod layout;
pub mod scene;
pub mod compiler;
pub mod animate;
pub mod ipc;
pub mod platform;
pub mod scripting;
pub mod devtools;
pub mod capabilities;
pub mod tray;
pub mod instance;
pub mod config;
pub mod embed;
pub mod theming;
pub mod run;
pub mod api;

/// Re-export key types for external consumers.
pub use prd::document::PrdDocument;

pub use gpu::context::GpuContext;
pub use scene::graph::SceneGraph;
pub use scene::input_handler::{InputHandler, RawInputEvent, UiEvent};
pub use scene::app_host::{AppHost, AppEvent, OpenDesktopAppBuilder};
pub use compiler::editable::EditableContext;
pub use devtools::DevTools;
pub use capabilities::{
    CapabilitySet, NetworkAccess, StorageAccess, IpcAccess, SystemInfo, FileSystemAccess,
    DeviceAccess, TrayAccess, Logging as LoggingCapability, Theming as ThemingCapability,
};
pub use tray::{SystemTray, TrayConfig, TrayMenu, TrayMenuEntry, TrayMenuItem, TrayItemStack, TraySubmenu, TrayMenuAction, TrayEvent};

// High-level convenience API.
pub use api::{run, start_with, Prism, PRISM, StartError};
pub use config::{PrismConfig, DefaultConfig, PagesConfig, WindowConfig, LoggingConfig, InstallConfig, ConfigError};
pub use embed::EmbeddedApp;

