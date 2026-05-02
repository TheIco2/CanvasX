// prism-runtime/src/compiler/mod.rs
//
// Prism HTML/CSS → PRD compiler.
// Converts a restricted subset of HTML+CSS into a compiled PRD document.
// This runs at load time (not per-frame), and the result is cached to disk.

pub mod html;
pub mod css;
pub mod bundle;
pub mod editable;

