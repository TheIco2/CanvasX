// openrender-runtime/src/layout/mod.rs
//
// Simplified layout engine.
// Supports: block flow, flexbox (row/column), absolute positioning.
// Does NOT support: CSS grid, floats, tables, inline layout (text is block).

pub mod engine;
pub mod flex;
pub mod types;
