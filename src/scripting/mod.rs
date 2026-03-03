// canvasx-runtime/src/scripting/mod.rs
//
// JavaScript runtime integration for CanvasX.
//
// Uses boa_engine (pure-Rust JS engine) to execute user scripts.
// Provides DOM-like API, Canvas 2D rendering via tiny-skia,
// and generic IPC bridge for host application communication.
//
// Architecture:
//   HTML <script> tags → collected during compilation
//   JsRuntime::new()  → creates boa Context with DOM/Canvas/IPC globals
//   JsRuntime::tick()  → runs requestAnimationFrame callbacks
//   Canvas pixmaps     → uploaded to wgpu textures each frame

pub mod runtime;
pub mod canvas2d;

pub use runtime::JsRuntime;
