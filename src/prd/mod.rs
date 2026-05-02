// prism-runtime/src/prd/mod.rs
//
// CXRD — Prism Runtime Document
//
// The compiled, binary-serializable scene graph format.
// HTML/CSS/JS gets compiled to PRD once, then the runtime only
// consumes PRD documents. No parsing at render time.

pub mod document;
pub mod node;
pub mod style;
pub mod animation;
pub mod asset;
pub mod value;
pub mod input;

