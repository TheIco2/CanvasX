// canvasx-runtime/src/cxrd/node.rs
//
// Scene graph node types for the CXRD format.
// Each node is a renderable element in the UI tree.

use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::cxrd::style::ComputedStyle;
use crate::cxrd::input::InputKind;
use crate::cxrd::value::Rect;

/// Unique node identifier within a CXRD document.
pub type NodeId = u32;

/// The kind of content a node holds.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    /// Container (div-like) — only has children, no intrinsic content.
    Container,

    /// Text content.
    Text {
        content: String,
    },

    /// Image element.
    Image {
        /// Index into the CXRD asset table.
        asset_index: u32,
        /// Object-fit style.
        fit: ImageFit,
    },

    /// SVG path (for inline SVGs used in CanvasX UI).
    SvgPath {
        /// SVG path data string.
        d: String,
        stroke_color: Option<[f32; 4]>,
        fill_color: Option<[f32; 4]>,
        stroke_width: f32,
    },

    /// HTML <canvas> element — pixels rendered by the JS runtime.
    /// The Canvas 2D context writes into a tiny-skia Pixmap which is
    /// uploaded to a GPU texture each frame.
    Canvas {
        width: u32,
        height: u32,
    },

    /// Scroll container.
    ScrollContainer {
        scroll_x: bool,
        scroll_y: bool,
    },

    /// Interactive input widget (button, text field, slider, etc.).
    /// These make CanvasX documents usable as full application windows.
    Input(InputKind),

    /// Page content container — children are swapped dynamically on navigation.
    /// Used with `<page-content default="...">` tags in HTML templates.
    PageContent,
}

/// Image fit mode (analogous to CSS object-fit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImageFit {
    Fill,
    Contain,
    Cover,
    ScaleDown,
    None,
}

impl Default for ImageFit {
    fn default() -> Self {
        ImageFit::Cover
    }
}

/// A single node in the CXRD scene graph.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CxrdNode {
    /// Unique ID within this document.
    pub id: NodeId,

    /// Optional string tag (for debugging / data-attribute mapping).
    pub tag: Option<String>,

    /// HTML id attribute (for getElementById lookups from JS).
    pub html_id: Option<String>,

    /// CSS class names (for state-based style switching).
    pub classes: Vec<String>,

    /// HTML attributes (data-*, aria-*, custom attributes, etc.).
    pub attributes: HashMap<String, String>,

    /// The type of content this node holds.
    pub kind: NodeKind,

    /// Fully computed style (resolved from CSS at compile time).
    pub style: ComputedStyle,

    /// Child node IDs (indexes into the document's node list).
    pub children: Vec<NodeId>,

    /// Optional event handlers (compiled from JS).
    pub events: Vec<EventBinding>,

    /// Animation references (indexes into document Animation table).
    pub animations: Vec<u32>,

    /// Layout result — populated after layout pass.
    #[serde(skip)]
    pub layout: LayoutResult,
}

/// Result of the layout pass for a single node.
#[derive(Debug, Clone, Copy, Default)]
pub struct LayoutResult {
    /// Final position and size in pixel-space (relative to viewport origin).
    pub rect: Rect,
    /// Content box (rect minus padding and border).
    pub content_rect: Rect,
    /// Clip rect from nearest overflow:hidden ancestor.
    pub clip: Option<Rect>,
    /// Resolved padding in px.
    pub padding: crate::cxrd::value::EdgeInsets,
    /// Resolved margin in px.
    pub margin: crate::cxrd::value::EdgeInsets,
}

/// An event binding compiled from JS.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventBinding {
    /// Event type (e.g., "click", "hover", "scroll").
    pub event: String,
    /// Action to perform.
    pub action: EventAction,
}

/// Actions that can be triggered by events.
/// These are the limited set of things the runtime supports
/// (no arbitrary JS execution).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum EventAction {
    /// Toggle a CSS class on a target node.
    ToggleClass { target: NodeId, class: String },
    /// Set a CSS class on a target node.
    SetClass { target: NodeId, class: String },
    /// Remove a CSS class from a target node.
    RemoveClass { target: NodeId, class: String },
    /// Navigate to a different scene/page.
    Navigate { scene_id: String },
    /// Send an IPC command to host application.
    IpcCommand { ns: String, cmd: String, args: Option<serde_json::Value> },
    /// Start an animation.
    StartAnimation { animation_index: u32 },
    /// Set scroll position.
    ScrollTo { target: NodeId, x: f32, y: f32 },
}

impl CxrdNode {
    /// Create a new container node.
    pub fn container(id: NodeId) -> Self {
        Self {
            id,
            tag: None,
            html_id: None,
            classes: Vec::new(),
            attributes: HashMap::new(),
            kind: NodeKind::Container,
            style: ComputedStyle::default(),
            children: Vec::new(),
            events: Vec::new(),
            animations: Vec::new(),
            layout: LayoutResult::default(),
        }
    }

    /// Create a new text node.
    pub fn text(id: NodeId, content: impl Into<String>) -> Self {
        Self {
            id,
            tag: None,
            html_id: None,
            classes: Vec::new(),
            attributes: HashMap::new(),
            kind: NodeKind::Text { content: content.into() },
            style: ComputedStyle::default(),
            children: Vec::new(),
            events: Vec::new(),
            animations: Vec::new(),
            layout: LayoutResult::default(),
        }
    }
}
