// openrender-runtime/src/compiler/css/mod.rs
//
// CSS compiler module — versioned CSS specification support.
//
// Architecture:
//   mod.rs      — Core types (CssRule, selectors) + parsing dispatch + re-exports
//   version.rs  — CssVersion trait and CssLevel enum
//   css3.rs     — Full CSS Level 3 specification implementation
//   parsing.rs  — Shared value parsers (dimensions, colors, gradients, etc.)
//
// To add CSS4 support:
//   1. Create `css4.rs` with `pub struct Css4;` implementing `CssVersion`
//   2. Add `CssLevel::Css4` variant to `version.rs`
//   3. Add dispatch arm in `apply_property()` below

pub mod version;
pub mod css3;
pub mod parsing;

// Re-export commonly used items for backward compatibility.
// Existing code that imports `crate::compiler::css::apply_property` etc. will
// continue to work without changes.
pub use parsing::{parse_color, parse_dimension, resolve_var_pub, parse_px};
pub use version::{CssVersion, CssLevel, PseudoClassCategory};

use crate::cxrd::style::*;
use std::collections::HashMap;

// ═════════════════════════════════════════════════════════════════════════════
//  CORE TYPES
// ═════════════════════════════════════════════════════════════════════════════

/// A parsed CSS rule.
#[derive(Debug, Clone)]
pub struct CssRule {
    /// Selector string.
    pub selector: String,
    /// Parsed complex selector (list of compound selectors for descendant matching).
    pub compound_selectors: Vec<CompoundSelector>,
    /// Property declarations.
    pub declarations: Vec<(String, String)>,
    /// Pseudo-class that must be active for this rule to apply (e.g. "hover", "focus").
    /// `None` means the rule applies unconditionally.
    pub pseudo_class: Option<String>,
}

/// A compound CSS selector part (matches a single element).
/// Multiple conditions must ALL match the same element.
#[derive(Debug, Clone, PartialEq)]
pub struct CompoundSelector {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub id: Option<String>,
    pub is_universal: bool,
}

impl CompoundSelector {
    /// Check if this compound selector matches a node.
    pub fn matches_node(&self, tag: Option<&str>, classes: &[String], html_id: Option<&str>) -> bool {
        if self.is_universal {
            return true;
        }
        if let Some(ref t) = self.tag {
            if tag != Some(t.as_str()) {
                return false;
            }
        }
        for cls in &self.classes {
            if !classes.contains(cls) {
                return false;
            }
        }
        if let Some(ref id) = self.id {
            if html_id != Some(id.as_str()) {
                return false;
            }
        }
        // Must have at least one condition
        self.tag.is_some() || !self.classes.is_empty() || self.id.is_some()
    }
}

/// A part of a CSS selector (kept for backward compatibility).
#[derive(Debug, Clone, PartialEq)]
pub enum SelectorPart {
    Tag(String),
    Class(String),
    Id(String),
    Universal,
}

// ═════════════════════════════════════════════════════════════════════════════
//  VERSION DISPATCH
// ═════════════════════════════════════════════════════════════════════════════

/// Apply a CSS property value to a ComputedStyle.
/// Dispatches to the active CSS version (currently CSS3).
pub fn apply_property(style: &mut ComputedStyle, property: &str, value: &str, variables: &HashMap<String, String>) {
    // Dispatch to CSS3 by default.
    // When CSS4 is added, this can accept a CssLevel parameter or be
    // configured at the document/compiler level.
    if !css3::Css3::apply_property(style, property, value, variables) {
        log::debug!("Unsupported CSS property: {}", property);
    }
}

/// Check whether a pseudo-class name is recognized by the active CSS version.
pub fn is_pseudo_class(name: &str) -> bool {
    css3::Css3::is_pseudo_class(name)
}

/// Check whether a pseudo-element name is recognized by the active CSS version.
pub fn is_pseudo_element(name: &str) -> bool {
    css3::Css3::is_pseudo_element(name)
}

/// Check whether an at-rule name is recognized by the active CSS version.
pub fn is_at_rule(name: &str) -> bool {
    css3::Css3::is_at_rule(name)
}

/// Classify a pseudo-class for runtime behavior.
pub fn pseudo_class_category(name: &str) -> PseudoClassCategory {
    css3::Css3::pseudo_class_category(name)
}

// ═════════════════════════════════════════════════════════════════════════════
//  CSS PARSING (core parser — shared across all versions)
// ═════════════════════════════════════════════════════════════════════════════

/// Parse CSS source into a list of rules.
pub fn parse_css(source: &str) -> Vec<CssRule> {
    let mut rules = Vec::new();
    let mut pos = 0;
    let source = strip_comments(source);
    let bytes = source.as_bytes();

    while pos < bytes.len() {
        while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }

        // Skip @rules
        if bytes[pos] == b'@' {
            if let Some(block_start) = source[pos..].find('{') {
                let abs_start = pos + block_start;
                if let Some(block_end) = find_matching_brace(&source, abs_start) {
                    let at_rule = &source[pos..abs_start].trim();
                    if at_rule.starts_with("@keyframes") {
                        // TODO: Parse keyframes into AnimationDef
                    }
                    pos = block_end + 1;
                    continue;
                }
            }
            if let Some(semi) = source[pos..].find(';') {
                pos += semi + 1;
            } else {
                break;
            }
            continue;
        }

        // Parse selector
        let selector_start = pos;
        while pos < bytes.len() && bytes[pos] != b'{' {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let selector = source[selector_start..pos].trim().to_string();
        pos += 1; // skip '{'

        // Parse declarations until '}'
        let decl_start = pos;
        let mut depth = 1;
        while pos < bytes.len() && depth > 0 {
            if bytes[pos] == b'{' { depth += 1; }
            if bytes[pos] == b'}' { depth -= 1; }
            if depth > 0 { pos += 1; }
        }
        let decl_block = &source[decl_start..pos];
        pos += 1; // skip '}'

        let declarations = parse_declarations(decl_block);

        // Handle comma-separated selectors
        let selector_group: Vec<&str> = selector.split(',').map(|s| s.trim()).filter(|s| !s.is_empty()).collect();
        for sel in selector_group {
            let (compound_selectors, pseudo_class) = parse_compound_selector(sel);
            rules.push(CssRule {
                selector: sel.to_string(),
                compound_selectors,
                declarations: declarations.clone(),
                pseudo_class,
            });
        }
    }

    rules
}

/// Parse a CSS selector string into a chain of compound selectors.
/// Returns `(compound_selectors, pseudo_class)`.
fn parse_compound_selector(selector: &str) -> (Vec<CompoundSelector>, Option<String>) {
    let mut chain = Vec::new();
    let mut pseudo_class: Option<String> = None;

    for token in selector.split_whitespace() {
        let token = if token.contains("::") {
            return (Vec::new(), None);
        } else if let Some(idx) = token.find(':') {
            let pseudo = &token[idx + 1..];
            let before = &token[..idx];
            if before.is_empty() {
                chain.push(CompoundSelector {
                    tag: Some(token.to_string()),
                    classes: Vec::new(),
                    id: None,
                    is_universal: false,
                });
                continue;
            }
            pseudo_class = Some(pseudo.to_lowercase());
            before
        } else {
            token
        };

        if token.is_empty() {
            continue;
        }

        if token == "*" {
            chain.push(CompoundSelector {
                tag: None, classes: Vec::new(), id: None, is_universal: true,
            });
            continue;
        }

        let mut compound = CompoundSelector {
            tag: None, classes: Vec::new(), id: None, is_universal: false,
        };

        let mut pos = 0;
        let chars: Vec<char> = token.chars().collect();
        while pos < chars.len() {
            match chars[pos] {
                '.' => {
                    pos += 1;
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.classes.push(chars[start..pos].iter().collect());
                    }
                }
                '#' => {
                    pos += 1;
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.id = Some(chars[start..pos].iter().collect());
                    }
                }
                _ => {
                    let start = pos;
                    while pos < chars.len() && chars[pos] != '.' && chars[pos] != '#' {
                        pos += 1;
                    }
                    if pos > start {
                        compound.tag = Some(chars[start..pos].iter().collect::<String>().to_lowercase());
                    }
                }
            }
        }

        chain.push(compound);
    }
    (chain, pseudo_class)
}

/// Parse a CSS selector into parts (legacy, kept for backward compat).
#[allow(dead_code)]
fn parse_selector(selector: &str) -> Vec<SelectorPart> {
    let mut parts = Vec::new();
    for token in selector.split_whitespace() {
        if token == "*" {
            parts.push(SelectorPart::Universal);
        } else if let Some(class) = token.strip_prefix('.') {
            parts.push(SelectorPart::Class(class.to_string()));
        } else if let Some(id) = token.strip_prefix('#') {
            parts.push(SelectorPart::Id(id.to_string()));
        } else {
            parts.push(SelectorPart::Tag(token.to_lowercase()));
        }
    }
    parts
}

/// Parse declaration block into (property, value) pairs.
fn parse_declarations(block: &str) -> Vec<(String, String)> {
    let mut decls = Vec::new();
    for decl in block.split(';') {
        let decl = decl.trim();
        if decl.is_empty() {
            continue;
        }
        if let Some((prop, val)) = decl.split_once(':') {
            decls.push((prop.trim().to_lowercase(), val.trim().to_string()));
        }
    }
    decls
}

// ═════════════════════════════════════════════════════════════════════════════
//  INTERNAL HELPERS
// ═════════════════════════════════════════════════════════════════════════════

/// Strip CSS comments.
fn strip_comments(source: &str) -> String {
    let mut result = String::with_capacity(source.len());
    let mut in_comment = false;
    let bytes = source.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if !in_comment && i + 1 < bytes.len() && bytes[i] == b'/' && bytes[i + 1] == b'*' {
            in_comment = true;
            i += 2;
        } else if in_comment && i + 1 < bytes.len() && bytes[i] == b'*' && bytes[i + 1] == b'/' {
            in_comment = false;
            i += 2;
        } else if !in_comment {
            result.push(bytes[i] as char);
            i += 1;
        } else {
            i += 1;
        }
    }

    result
}

/// Find the matching closing brace for an opening brace at `start`.
fn find_matching_brace(source: &str, start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0;
    let mut i = start;
    while i < bytes.len() {
        if bytes[i] == b'{' { depth += 1; }
        if bytes[i] == b'}' {
            depth -= 1;
            if depth == 0 {
                return Some(i);
            }
        }
        i += 1;
    }
    None
}
