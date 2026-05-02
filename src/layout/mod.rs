// prism-runtime/src/layout/mod.rs
//
// Simplified layout engine.
// Supports: block flow, flexbox (row/column, wrap, order, align-content),
//           CSS grid (template columns/rows, auto-placement), absolute/fixed/relative positioning.
// Does NOT support: floats, tables, inline layout (text is block).

pub mod engine;
pub mod flex;
pub mod types;

