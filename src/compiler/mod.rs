// openrender-runtime/src/compiler/mod.rs
//
// OpenRender HTML/CSS → CXRD compiler.
// Converts a restricted subset of HTML+CSS into a compiled CXRD document.
// This runs at load time (not per-frame), and the result is cached to disk.

pub mod html;
pub mod css;
pub mod bundle;
pub mod editable;
