// canvasx-runtime — GPU-native deterministic UI runtime for CanvasX
//
// Architecture:
//   HTML/CSS/JS → Compiler → CXRD (CanvasX Runtime Document) → GPU Renderer
//
// Supports: Vulkan, DirectX 12, DirectX 11 (via wgpu backends)
//
// This crate is the rendering engine binary. host application talks to it via
// IPC (named pipes) to push system data and control scenes (wallpapers,
// widgets, status bar).

pub mod logging;
pub mod cxrd;
pub mod cxrp;
pub mod cxrl;
pub mod gpu;
pub mod layout;
pub mod scene;
pub mod compiler;
pub mod animate;
pub mod ipc;
pub mod platform;
pub mod scripting;

/// Re-export key types for external consumers.
pub use cxrd::document::CxrdDocument;
pub use cxrp::loader::LoadedPackage;
pub use cxrl::loader::LoadedLibrary;
pub use gpu::context::GpuContext;
pub use scene::graph::SceneGraph;
pub use scene::input_handler::{InputHandler, RawInputEvent, UiEvent};
pub use scene::app_host::{AppHost, AppEvent, SentinelAppBuilder};
pub use compiler::editable::EditableContext;
