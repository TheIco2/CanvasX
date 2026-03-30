// openrender-runtime/src/compiler/editable.rs
//
// Editable properties system — bridges OpenDesktop's manifest.json editable
// schema + editable.yaml user overrides into a flat property map that
// OpenRender scenes can consume.
//
// In the WebView2 world, editables became CSS variables pushed into JS.
// In OpenRender, they become data-bound scene properties applied at runtime.
//
// Schema format (from manifest.json):
//   { "editable": { "groupKey": { "name": "..", "propKey": { "selector": "..", "value": .., "variable": "--css-var" } } } }
//
// Override format (editable.yaml):
//   Same nested structure but only contains user-modified values.

use std::collections::HashMap;
use std::path::Path;
use serde_json::Value;

/// A resolved editable property with metadata.
#[derive(Debug, Clone)]
pub struct EditableProperty {
    /// The CSS variable name (e.g. "--accent").
    pub variable: String,
    /// Resolved value (user override if present, else manifest default).
    pub value: EditableValue,
    /// The selector (widget type) used in the options UI.
    pub selector: String,
    /// For sliders: min, max, step constraints.
    pub constraints: Option<SliderConstraints>,
    /// For dropdowns: available options.
    pub options: Option<Vec<DropdownOption>>,
    /// Path in the nested structure (e.g. "accentSettings.accent").
    pub path: String,
    /// Group name (first level key).
    pub group: String,
    /// Human-readable group name ("Accent Colors").
    pub group_name: Option<String>,
    /// Group description.
    pub group_description: Option<String>,
}

/// A resolved value from the editable system.
#[derive(Debug, Clone)]
pub enum EditableValue {
    String(String),
    Number(f64),
    Bool(bool),
}

impl EditableValue {
    pub fn as_string(&self) -> String {
        match self {
            EditableValue::String(s) => s.clone(),
            EditableValue::Number(n) => {
                // Format without trailing zeros for integers.
                if *n == (*n as i64) as f64 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            EditableValue::Bool(b) => b.to_string(),
        }
    }

    pub fn as_css_value(&self) -> String {
        match self {
            EditableValue::String(s) => s.clone(),
            EditableValue::Number(n) => {
                if *n == (*n as i64) as f64 {
                    format!("{}", *n as i64)
                } else {
                    format!("{}", n)
                }
            }
            EditableValue::Bool(b) => if *b { "1".into() } else { "0".into() },
        }
    }
}

/// Constraints for slider properties.
#[derive(Debug, Clone)]
pub struct SliderConstraints {
    pub min: f64,
    pub max: f64,
    pub step: f64,
}

/// An option in a dropdown selector.
#[derive(Debug, Clone)]
pub struct DropdownOption {
    pub name: String,
    pub selector: Option<String>,
    pub value: String,
}

/// Resolved editable context — all properties with current values.
pub struct EditableContext {
    /// All resolved properties, keyed by CSS variable name.
    pub properties: HashMap<String, EditableProperty>,
    /// Flat CSS variable map (variable → value string) for scene injection.
    pub css_vars: HashMap<String, String>,
}

impl EditableContext {
    /// Load editables from a manifest.json path and an optional editable.yaml path.
    pub fn load(manifest_path: &Path, overrides_path: Option<&Path>) -> Result<Self, String> {
        let manifest_str = std::fs::read_to_string(manifest_path)
            .map_err(|e| format!("Failed to read manifest: {}", e))?;
        let manifest: Value = serde_json::from_str(&manifest_str)
            .map_err(|e| format!("Invalid manifest JSON: {}", e))?;

        let overrides = if let Some(yaml_path) = overrides_path {
            if yaml_path.exists() {
                let yaml_str = std::fs::read_to_string(yaml_path)
                    .map_err(|e| format!("Failed to read editable.yaml: {}", e))?;

                // Parse YAML into JSON Value for uniform handling.
                // We do a simple YAML→JSON conversion via serde_yaml if available,
                // or fall back to our minimal parser.
                parse_yaml_to_json(&yaml_str)?
            } else {
                Value::Object(serde_json::Map::new())
            }
        } else {
            Value::Object(serde_json::Map::new())
        };

        let editable_root = manifest.get("editable")
            .ok_or_else(|| "No 'editable' key in manifest".to_string())?;

        let mut properties = HashMap::new();
        let mut css_vars = HashMap::new();

        if let Value::Object(groups) = editable_root {
            for (group_key, group_val) in groups {
                extract_group(
                    group_key,
                    group_val,
                    &overrides,
                    &mut properties,
                    &mut css_vars,
                );
            }
        }

        Ok(Self { properties, css_vars })
    }

    /// Reload overrides from a YAML file (e.g., after user edits).
    pub fn apply_overrides(&mut self, overrides_yaml: &str) -> Result<(), String> {
        let overrides = parse_yaml_to_json(overrides_yaml)?;
        // Re-resolve all properties with new overrides.
        for prop in self.properties.values_mut() {
            if let Some(new_val) = resolve_override_value(&overrides, &prop.path) {
                prop.value = new_val.clone();
                self.css_vars.insert(prop.variable.clone(), new_val.as_css_value());
            }
        }
        Ok(())
    }

    /// Generate a YAML string from current values (for saving back to editable.yaml).
    pub fn to_yaml(&self) -> String {
        let mut lines = Vec::new();
        // Group by group name.
        let mut groups: HashMap<&str, Vec<&EditableProperty>> = HashMap::new();
        for prop in self.properties.values() {
            groups.entry(&prop.group).or_default().push(prop);
        }

        let mut sorted_groups: Vec<_> = groups.into_iter().collect();
        sorted_groups.sort_by_key(|(k, _)| *k);

        for (group_key, props) in sorted_groups {
            lines.push(format!("{}:", group_key));
            // Add group metadata if present.
            if let Some(ref name) = props.first().and_then(|p| p.group_name.as_ref()) {
                lines.push(format!("  name: {}", name));
            }
            if let Some(ref desc) = props.first().and_then(|p| p.group_description.as_ref()) {
                lines.push(format!("  description: {}", desc));
            }
            for prop in &props {
                // Extract the property key (last segment of path).
                let prop_key = prop.path.rsplit('.').next().unwrap_or(&prop.path);
                lines.push(format!("  {}:", prop_key));
                lines.push(format!("    selector: {}", prop.selector));
                lines.push(format!("    value: {}", prop.value.as_css_value()));
                lines.push(format!("    variable: {}", prop.variable));
                if let Some(ref c) = prop.constraints {
                    lines.push(format!("    min: {}", c.min));
                    lines.push(format!("    max: {}", c.max));
                    lines.push(format!("    step: {}", c.step));
                }
            }
            lines.push(String::new());
        }

        lines.join("\n")
    }
}

// --- Internal helpers ---

fn extract_group(
    group_key: &str,
    group_val: &Value,
    overrides: &Value,
    properties: &mut HashMap<String, EditableProperty>,
    css_vars: &mut HashMap<String, String>,
) {
    // Check if this is a direct property (has "selector" key).
    if group_val.get("selector").is_some() {
        extract_property(group_key, group_key, group_val, overrides, None, None, properties, css_vars);
        return;
    }

    // Otherwise it's a group with nested properties.
    let group_name = group_val.get("name").and_then(|v| v.as_str()).map(String::from);
    let group_desc = group_val.get("description").and_then(|v| v.as_str()).map(String::from);

    if let Value::Object(props) = group_val {
        for (prop_key, prop_val) in props {
            // Skip metadata keys.
            if prop_key == "name" || prop_key == "description" {
                continue;
            }
            let path = format!("{}.{}", group_key, prop_key);
            extract_property(
                &path,
                group_key,
                prop_val,
                overrides,
                group_name.as_deref(),
                group_desc.as_deref(),
                properties,
                css_vars,
            );
        }
    }
}

fn extract_property(
    path: &str,
    group_key: &str,
    prop_val: &Value,
    overrides: &Value,
    group_name: Option<&str>,
    group_desc: Option<&str>,
    properties: &mut HashMap<String, EditableProperty>,
    css_vars: &mut HashMap<String, String>,
) {
    let selector = match prop_val.get("selector").and_then(|v| v.as_str()) {
        Some(s) => s.to_string(),
        None => return, // Not a property.
    };

    let variable = match prop_val.get("variable").and_then(|v| v.as_str()) {
        Some(v) => v.to_string(),
        None => return, // No CSS variable binding.
    };

    // Manifest default value.
    let default_value = json_to_editable_value(prop_val.get("value"));

    // Attempt to find override.
    let override_value = resolve_override_value(overrides, path);
    let resolved_value = override_value.unwrap_or(default_value);

    // Constraints for sliders.
    let constraints = if selector == "slider" {
        Some(SliderConstraints {
            min: prop_val.get("min").and_then(|v| v.as_f64()).unwrap_or(0.0),
            max: prop_val.get("max").and_then(|v| v.as_f64()).unwrap_or(100.0),
            step: prop_val.get("step").and_then(|v| v.as_f64()).unwrap_or(1.0),
        })
    } else {
        None
    };

    // Options for dropdowns.
    let options = if selector == "dropdown" {
        prop_val.get("options").and_then(|v| v.as_array()).map(|arr| {
            arr.iter().filter_map(|item| {
                let name = item.get("name").and_then(|v| v.as_str())?.to_string();
                let sel = item.get("selector").and_then(|v| v.as_str()).map(String::from);
                let value = item.get("value").and_then(|v| v.as_str()).unwrap_or_default().to_string();
                Some(DropdownOption { name, selector: sel, value })
            }).collect()
        })
    } else {
        None
    };

    css_vars.insert(variable.clone(), resolved_value.as_css_value());

    properties.insert(variable.clone(), EditableProperty {
        variable,
        value: resolved_value,
        selector,
        constraints,
        options,
        path: path.to_string(),
        group: group_key.to_string(),
        group_name: group_name.map(String::from),
        group_description: group_desc.map(String::from),
    });
}

fn resolve_override_value(overrides: &Value, path: &str) -> Option<EditableValue> {
    let parts: Vec<&str> = path.split('.').collect();
    let mut current = overrides;

    for part in &parts {
        current = current.get(part)?;
    }

    // The override node could have a "value" key, or be the value itself.
    if let Some(val) = current.get("value") {
        Some(json_to_editable_value(Some(val)))
    } else if current.is_object() {
        None // It's a group, not a value.
    } else {
        Some(json_to_editable_value(Some(current)))
    }
}

fn json_to_editable_value(val: Option<&Value>) -> EditableValue {
    match val {
        Some(Value::String(s)) => EditableValue::String(s.clone()),
        Some(Value::Number(n)) => EditableValue::Number(n.as_f64().unwrap_or(0.0)),
        Some(Value::Bool(b)) => EditableValue::Bool(*b),
        _ => EditableValue::String(String::new()),
    }
}

/// Minimal YAML-to-JSON parser for editable.yaml files.
///
/// Handles the subset of YAML that OpenDesktop's editable files actually use:
///   - Nested object keys via indentation
///   - Scalar values (strings, numbers, bools)
///   - Simple arrays (- item lines)
///
/// This avoids pulling in a full serde_yaml dependency.
fn parse_yaml_to_json(yaml: &str) -> Result<Value, String> {
    let mut root = serde_json::Map::new();
    let mut stack: Vec<(usize, String, serde_json::Map<String, Value>)> = Vec::new();

    for line in yaml.lines() {
        // Skip blank lines and comments.
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let indent = line.len() - line.trim_start().len();

        // Pop stack to appropriate level.
        while let Some((lvl, _, _)) = stack.last() {
            if indent <= *lvl {
                let (_, key, map) = stack.pop().unwrap();
                let target = if let Some((_, _, parent)) = stack.last_mut() {
                    parent
                } else {
                    &mut root
                };
                target.insert(key, Value::Object(map));
            } else {
                break;
            }
        }

        if let Some(colon_pos) = trimmed.find(':') {
            let key = trimmed[..colon_pos].trim().to_string();
            let after = trimmed[colon_pos + 1..].trim();

            if after.is_empty() {
                // Nested object.
                stack.push((indent, key, serde_json::Map::new()));
            } else {
                // Scalar value.
                let value = parse_yaml_scalar(after);
                let target = if let Some((_, _, map)) = stack.last_mut() {
                    map
                } else {
                    &mut root
                };
                target.insert(key, value);
            }
        } else if trimmed.starts_with("- ") {
            // Array item — simplified handling.
            let item = parse_yaml_scalar(trimmed[2..].trim());
            // Arrays are appended to the last stack entry's last key.
            if let Some((_, _key, map)) = stack.last_mut() {
                // Find or create the array on the parent.
                let arr_key = "__array__".to_string();
                let arr = map.entry(arr_key).or_insert_with(|| Value::Array(Vec::new()));
                if let Value::Array(ref mut vec) = arr {
                    vec.push(item);
                }
            }
        }
    }

    // Unwind remaining stack.
    while let Some((_, key, map)) = stack.pop() {
        let target = if let Some((_, _, parent)) = stack.last_mut() {
            parent
        } else {
            &mut root
        };
        target.insert(key, Value::Object(map));
    }

    Ok(Value::Object(root))
}

fn parse_yaml_scalar(s: &str) -> Value {
    // Bool.
    if s == "true" || s == "yes" {
        return Value::Bool(true);
    }
    if s == "false" || s == "no" {
        return Value::Bool(false);
    }
    // Number.
    if let Ok(n) = s.parse::<i64>() {
        return Value::Number(n.into());
    }
    if let Ok(n) = s.parse::<f64>() {
        if let Some(num) = serde_json::Number::from_f64(n) {
            return Value::Number(num);
        }
    }
    // String (strip optional quotes).
    let unquoted = s.trim_matches('"').trim_matches('\'');
    Value::String(unquoted.to_string())
}
