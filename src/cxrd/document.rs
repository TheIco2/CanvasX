// openrender-runtime/src/cxrd/document.rs
//
// The top-level CXRD document — a complete, self-contained renderable scene.
// This is what gets cached to disk and loaded by the runtime.

use serde::{Serialize, Deserialize};
use crate::cxrd::node::{CxrdNode, NodeId};
use crate::cxrd::animation::AnimationDef;
use crate::cxrd::asset::AssetBundle;
use crate::cxrd::value::Color;

/// Magic bytes at the start of a binary CXRD file.
pub const CXRD_MAGIC: &[u8; 4] = b"CXR\x01";

/// The version of the CXRD format.
pub const CXRD_VERSION: u32 = 1;

/// A complete OpenRender Runtime Document.
///
/// Contains everything needed to render a scene:
/// - Node tree (the scene graph)
/// - Animation definitions
/// - Bundled assets (images, fonts)
/// - CSS custom properties (resolved)
/// - Metadata
///
/// No external dependencies. No network. No dynamic parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CxrdDocument {
    /// Format version.
    pub version: u32,

    /// Document metadata.
    pub meta: CxrdMeta,

    /// The flat node list. Node 0 is always the root.
    pub nodes: Vec<CxrdNode>,

    /// Root node ID (usually 0).
    pub root: NodeId,

    /// Free node IDs available for reuse (prevents unbounded growth from innerHTML).
    #[serde(skip)]
    pub free_list: Vec<NodeId>,

    /// Animation definitions referenced by nodes.
    pub animations: Vec<AnimationDef>,

    /// Bundled assets.
    pub assets: AssetBundle,

    /// Resolved CSS custom properties (variables).
    pub variables: Vec<(String, String)>,

    /// Scene background color.
    pub background: Color,

    /// Viewport hint (the design resolution).
    pub viewport_width: f32,
    pub viewport_height: f32,

    /// Optional redirect target (from `<meta name="redirect" content="...">`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redirect: Option<String>,

    /// Document title extracted from `<title>` tags (last one wins).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    /// Icon paths declared via `<include type="icon">`.
    /// `target` is "window", "system", or "" (both).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub icons: Vec<IconDecl>,
}

/// An icon declaration from `<include type="icon" target="..." src="..." />`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IconDecl {
    /// "window", "system", "app", or "" (both window+system).
    pub target: String,
    /// Resolved filesystem path to the icon file.
    pub path: String,
    /// Asset index in the document's image bundle (set for target="app").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub asset_index: Option<u32>,
}

/// Document metadata.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CxrdMeta {
    /// Human-readable name.
    pub name: String,

    /// Original source path (for rebuild detection).
    pub source_path: Option<String>,

    /// SHA-256 hash of source files (for cache invalidation).
    pub source_hash: Option<String>,

    /// Scene type.
    pub scene_type: SceneType,

    /// Author info.
    pub author: Option<String>,
}

/// What kind of scene this document represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SceneType {
    /// Desktop wallpaper (renders behind desktop icons via WorkerW).
    Wallpaper,
    /// Status bar overlay.
    StatusBar,
    /// Widget (floating, composited over desktop).
    Widget,
    /// Configuration UI panel.
    ConfigPanel,
}

impl Default for SceneType {
    fn default() -> Self {
        SceneType::Wallpaper
    }
}

impl CxrdDocument {
    /// Create a new empty document.
    pub fn new(name: impl Into<String>, scene_type: SceneType) -> Self {
        let root = CxrdNode::container(0);
        Self {
            version: CXRD_VERSION,
            meta: CxrdMeta {
                name: name.into(),
                scene_type,
                ..Default::default()
            },
            nodes: vec![root],
            root: 0,
            free_list: Vec::new(),
            animations: Vec::new(),
            assets: AssetBundle::new(),
            variables: Vec::new(),
            background: Color::BLACK,
            viewport_width: 1920.0,
            viewport_height: 1080.0,
            redirect: None,
            title: None,
            icons: Vec::new(),
        }
    }

    /// Add a node to the document and return its ID.
    /// Reuses freed node slots when available to prevent unbounded growth.
    pub fn add_node(&mut self, mut node: CxrdNode) -> NodeId {
        if let Some(id) = self.free_list.pop() {
            node.id = id;
            self.nodes[id as usize] = node;
            id
        } else {
            let id = self.nodes.len() as NodeId;
            node.id = id;
            self.nodes.push(node);
            id
        }
    }

    /// Add a child to a parent node.
    pub fn add_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(p) = self.nodes.get_mut(parent as usize) {
            p.children.push(child);
        }
    }

    /// Get a node by ID.
    pub fn get_node(&self, id: NodeId) -> Option<&CxrdNode> {
        self.nodes.get(id as usize)
    }

    /// Get a mutable node by ID.
    pub fn get_node_mut(&mut self, id: NodeId) -> Option<&mut CxrdNode> {
        self.nodes.get_mut(id as usize)
    }

    /// Remove a child from a parent node.
    pub fn remove_child(&mut self, parent: NodeId, child: NodeId) {
        if let Some(p) = self.nodes.get_mut(parent as usize) {
            p.children.retain(|&c| c != child);
        }
    }

    /// Find the parent of a node (linear scan).
    pub fn find_parent(&self, child: NodeId) -> Option<NodeId> {
        for (i, node) in self.nodes.iter().enumerate() {
            if node.children.contains(&child) {
                return Some(i as NodeId);
            }
        }
        None
    }

    /// Find the first node with `NodeKind::PageContent`.
    pub fn find_page_content_node(&self) -> Option<NodeId> {
        use crate::cxrd::node::NodeKind;
        for (i, node) in self.nodes.iter().enumerate() {
            if matches!(node.kind, NodeKind::PageContent) {
                return Some(i as NodeId);
            }
        }
        None
    }

    /// Remove all children of a node recursively, adding freed IDs to the free list.
    pub fn free_subtree(&mut self, node_id: NodeId) {
        let children: Vec<NodeId> = self.nodes[node_id as usize].children.clone();
        for child_id in children {
            self.free_subtree_recursive(child_id);
        }
        self.nodes[node_id as usize].children.clear();
    }

    fn free_subtree_recursive(&mut self, node_id: NodeId) {
        let children: Vec<NodeId> = self.nodes[node_id as usize].children.clone();
        for child_id in children {
            self.free_subtree_recursive(child_id);
        }
        self.nodes[node_id as usize].children.clear();
        self.free_list.push(node_id);
    }

    /// Transplant all root children from another document into this one as children of `parent_id`.
    pub fn transplant_children_from(&mut self, source: &CxrdDocument, parent_id: NodeId) {
        let root_children: Vec<NodeId> = source.nodes[source.root as usize].children.clone();
        for &child_id in &root_children {
            let new_id = self.transplant_node_recursive(source, child_id);
            self.add_child(parent_id, new_id);
        }
    }

    fn transplant_node_recursive(&mut self, source: &CxrdDocument, src_id: NodeId) -> NodeId {
        let src_node = &source.nodes[src_id as usize];
        let src_children: Vec<NodeId> = src_node.children.clone();
        let mut new_node = src_node.clone();
        new_node.children = Vec::new();
        let new_id = self.add_node(new_node);
        for &child_src_id in &src_children {
            let child_new_id = self.transplant_node_recursive(source, child_src_id);
            self.add_child(new_id, child_new_id);
        }
        new_id
    }

    /// Serialize to binary for disk caching (.cxrd format).
    pub fn to_binary(&self) -> anyhow::Result<Vec<u8>> {
        let mut out = Vec::new();
        out.extend_from_slice(CXRD_MAGIC);
        out.extend_from_slice(&CXRD_VERSION.to_le_bytes());
        let body = serde_json::to_vec(self)?;
        out.extend_from_slice(&(body.len() as u64).to_le_bytes());
        out.extend(body);
        Ok(out)
    }

    /// Deserialize from binary (.cxrd format).
    pub fn from_binary(data: &[u8]) -> Result<Self, String> {
        if data.len() < 16 {
            return Err("CXRD file too small".into());
        }
        if &data[0..4] != CXRD_MAGIC {
            return Err("Invalid CXRD magic bytes".into());
        }
        let version = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        if version != CXRD_VERSION {
            return Err(format!("Unsupported CXRD version: {}", version));
        }
        let body_len = u64::from_le_bytes([
            data[8], data[9], data[10], data[11],
            data[12], data[13], data[14], data[15],
        ]) as usize;

        if data.len() < 16 + body_len {
            return Err("Truncated CXRD file".into());
        }

        serde_json::from_slice(&data[16..16 + body_len])
            .map_err(|e| format!("CXRD deserialize error: {}", e))
    }
}
