// prism-runtime/src/prd/input.rs
//
// Interactive input node types for building usable app windows.
// These extend the base NodeKind with interactive widgets that
// accept user input and maintain internal state.

use serde::{Serialize, Deserialize};
use crate::prd::value::Color;

/// Input widget types that can be used in OpenRender documents.
/// These provide the interactive primitives needed to build full app UIs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum InputKind {
    /// Single-line text input field.
    TextInput {
        /// Placeholder text shown when empty.
        placeholder: String,
        /// Current value.
        value: String,
        /// Maximum character length (0 = unlimited).
        max_length: u32,
        /// Whether the field is read-only.
        read_only: bool,
        /// Input type hint (text, password, number, email).
        input_type: TextInputType,
    },

    /// Multi-line text area.
    TextArea {
        placeholder: String,
        value: String,
        max_length: u32,
        read_only: bool,
        rows: u32,
    },

    /// Clickable button.
    Button {
        label: String,
        /// Whether the button is disabled.
        disabled: bool,
        /// Visual variant.
        variant: ButtonVariant,
    },

    /// Toggle checkbox / switch.
    Checkbox {
        label: String,
        checked: bool,
        disabled: bool,
        /// Visual style (checkbox or toggle switch).
        style: CheckboxStyle,
    },

    /// Numeric slider.
    Slider {
        value: f64,
        min: f64,
        max: f64,
        step: f64,
        disabled: bool,
        /// Whether to show a value label.
        show_value: bool,
    },

    /// Dropdown / select menu.
    Dropdown {
        /// Available options: (value, display_label).
        options: Vec<(String, String)>,
        /// Currently selected value.
        selected: Option<String>,
        /// Placeholder when nothing selected.
        placeholder: String,
        disabled: bool,
        /// Whether the dropdown is currently open.
        #[serde(skip)]
        open: bool,
    },

    /// Color picker.
    ColorPicker {
        value: Color,
        /// Whether alpha channel is editable.
        show_alpha: bool,
        disabled: bool,
    },

    /// Asset selector (file/image picker).
    AssetSelector {
        /// Currently selected asset path.
        value: String,
        /// Accepted file extensions.
        accept: Vec<String>,
        /// Label text.
        label: String,
    },

    /// Tab bar (for multi-page navigation).
    TabBar {
        /// Tab definitions: (id, label, icon_asset_index).
        tabs: Vec<TabDef>,
        /// Currently active tab ID.
        active_tab: String,
    },

    /// Scroll view — wraps children in a scrollable region.
    ScrollView {
        /// Allow horizontal scrolling.
        scroll_x: bool,
        /// Allow vertical scrolling.
        scroll_y: bool,
        /// Current scroll offset.
        #[serde(skip)]
        offset_x: f32,
        #[serde(skip)]
        offset_y: f32,
    },

    /// Link / anchor — clickable text that navigates or opens external URLs.
    Link {
        label: String,
        /// Target: scene ID, URL, or IPC command.
        href: LinkTarget,
    },
}

/// Text input type hints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TextInputType {
    Text,
    Password,
    Number,
    Email,
    Search,
}

/// Button visual variants.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ButtonVariant {
    Primary,
    Secondary,
    Danger,
    Ghost,
    Link,
}

/// Checkbox display styles.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CheckboxStyle {
    Checkbox,
    Toggle,
}

/// Tab definition.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TabDef {
    /// Unique tab identifier.
    pub id: String,
    /// Display label.
    pub label: String,
    /// Optional icon asset index.
    pub icon: Option<u32>,
    /// Whether this tab is closable.
    pub closable: bool,
}

/// Link target types.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum LinkTarget {
    /// Navigate to a scene within the current document/package.
    Scene(String),
    /// Open an external URL in the default browser.
    External(String),
    /// Trigger an IPC command.
    Ipc { ns: String, cmd: String, args: Option<serde_json::Value> },
}

/// Focus state for interactive elements.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusState {
    /// Not focused.
    None,
    /// Focused (keyboard input goes here).
    Focused,
    /// Hovered by mouse.
    Hovered,
    /// Being actively pressed.
    Active,
}

impl Default for FocusState {
    fn default() -> Self {
        FocusState::None
    }
}

/// Runtime state for interactive nodes. Not serialised — populated at runtime.
#[derive(Debug, Clone, Default)]
pub struct InteractionState {
    /// Current focus state.
    pub focus: FocusState,
    /// Whether the mouse is over this node.
    pub hovered: bool,
    /// Whether the primary mouse button is pressed on this node.
    pub pressed: bool,
    /// Cursor position for text inputs (character index).
    pub cursor_pos: usize,
    /// Selection range in text inputs (start, end).
    pub selection: Option<(usize, usize)>,
    /// Scroll offset for scroll views.
    pub scroll_x: f32,
    pub scroll_y: f32,
    /// Content size for scroll views (computed during layout).
    pub content_width: f32,
    pub content_height: f32,
}

