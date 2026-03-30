// openrender-runtime/src/cxrd/mod.rs
//
// CXRD — OpenRender Runtime Document
//
// The compiled, binary-serializable scene graph format.
// HTML/CSS/JS gets compiled to CXRD once, then the runtime only
// consumes CXRD documents. No parsing at render time.

pub mod document;
pub mod node;
pub mod style;
pub mod animation;
pub mod asset;
pub mod value;
pub mod input;
