// canvasx-runtime/src/compiler/css/version.rs
//
// CSS version abstraction layer.
// Each CSS version (CSS3, future CSS4, etc.) implements the CssVersion trait,
// providing version-specific property application, pseudo-class recognition,
// and at-rule handling. To add a new CSS version, create a new module (e.g.
// `css4.rs`) and implement this trait.

use crate::cxrd::style::ComputedStyle;
use std::collections::HashMap;

/// Which CSS specification level to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CssLevel {
    /// CSS Level 3 (current standard, module-based updates).
    #[default]
    Css3,
    // Future: Css4, if it ever ships as a distinct level.
}

/// Trait that each CSS version module must implement.
///
/// To add CSS4 support, create `css4.rs`, define `pub struct Css4;`,
/// implement `CssVersion for Css4`, and register it in `css/mod.rs`.
pub trait CssVersion {
    /// Apply a CSS property value to a ComputedStyle.
    /// Returns `true` if the property was recognized (even if not rendered).
    fn apply_property(
        style: &mut ComputedStyle,
        property: &str,
        value: &str,
        variables: &HashMap<String, String>,
    ) -> bool;

    /// Check if a pseudo-class name is recognized by this CSS version.
    /// `name` may include arguments, e.g. `"nth-child(2n+1)"`.
    fn is_pseudo_class(name: &str) -> bool;

    /// Check if a pseudo-element name is recognized by this CSS version.
    /// `name` may include `::` prefix or not.
    fn is_pseudo_element(name: &str) -> bool;

    /// Check if an at-rule name is recognized by this CSS version.
    fn is_at_rule(name: &str) -> bool;

    /// Classify a pseudo-class for runtime behavior.
    fn pseudo_class_category(name: &str) -> PseudoClassCategory;
}

/// How a pseudo-class is evaluated at runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PseudoClassCategory {
    /// Dynamic interaction states: :hover, :active, :focus, :focus-visible, :focus-within.
    Interactive,
    /// Structural position: :first-child, :last-child, :nth-child(), :only-child, etc.
    Structural,
    /// Form / input states: :checked, :disabled, :enabled, :valid, :invalid, :required, etc.
    FormState,
    /// Link states: :link, :visited, :any-link, :local-link.
    LinkState,
    /// Media / document states: :fullscreen, :picture-in-picture, :modal, etc.
    MediaState,
    /// Functional selectors that wrap other selectors: :is(), :not(), :has(), :where().
    Functional,
    /// Element definition / custom-element states: :defined, :host, :host-context, :state().
    ElementState,
    /// Unknown or not-yet-categorized.
    Unknown,
}
