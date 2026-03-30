// openrender-runtime/src/scene/mod.rs
//
// Scene graph execution — traverses the CXRD, produces GPU draw calls
// (UiInstance list) and text areas.

pub mod graph;
pub mod paint;
pub mod text;
pub mod input_handler;
pub mod app_host;
