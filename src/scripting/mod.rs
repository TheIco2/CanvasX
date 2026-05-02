// prism-runtime/src/scripting/mod.rs
//
// JavaScript runtime integration for OpenRender.
//
// Uses V8 (via the `v8` crate) for high-performance JS execution.
// Provides DOM-like API, Canvas 2D rendering via tiny-skia,
// and generic IPC bridge for host application communication.
//
// Architecture:
//   HTML <script> tags → collected during compilation
//   JsRuntime::new()  → creates V8 isolate with DOM/Canvas/IPC globals
//   JsRuntime::tick()  → runs requestAnimationFrame callbacks
//   Canvas pixmaps     → uploaded to wgpu textures each frame

pub mod v8_runtime;
pub mod canvas2d;
pub mod js_worker;

pub use v8_runtime::JsRuntime;
pub use js_worker::{JsWorkerHandle, JsWorkerInit, JsCommand, JsResult, DirtyCanvas};

