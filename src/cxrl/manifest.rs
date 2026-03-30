// openrender-runtime/src/cxrl/manifest.rs
//
// Library manifest — describes the contents of a .cxrl archive.
// Libraries are reusable collections of components, styles, animations, and assets
// that can be referenced by .cxrd documents and .cxrp packages.

use serde::{Serialize, Deserialize};

/// Current library format version.
pub const CXRL_FORMAT_VERSION: u32 = 1;

/// The manifest for a OpenRender Runtime Library (.cxrl).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryManifest {
    /// Format version.
    pub format_version: u32,

    /// Library metadata.
    pub metadata: LibraryMeta,

    /// Exported components (reusable node subtrees).
    pub components: Vec<ComponentEntry>,

    /// Exported style themes.
    pub themes: Vec<ThemeEntry>,

    /// Exported animation presets.
    pub animations: Vec<AnimationPresetEntry>,

    /// Bundled assets (fonts, icons, textures).
    pub assets: Vec<LibraryAsset>,

    /// Dependencies on other libraries.
    pub dependencies: Vec<LibraryDependency>,
}

/// Library metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryMeta {
    /// Unique library identifier (e.g., "openrender.ui-kit").
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Semantic version.
    pub version: String,

    /// Author.
    pub author: Option<String>,

    /// Description.
    pub description: Option<String>,

    /// License.
    pub license: Option<String>,

    /// Minimum OpenRender runtime version required.
    pub min_runtime_version: Option<String>,

    /// Tags for discovery.
    pub tags: Vec<String>,
}

/// A reusable component: a subtree of CXRD nodes that can be instantiated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentEntry {
    /// Component identifier (unique within this library).
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Category for organisation (e.g., "layout", "input", "display").
    pub category: Option<String>,

    /// Description of what this component does.
    pub description: Option<String>,

    /// Path to the serialised component data inside the archive.
    pub path: String,

    /// Configurable properties exposed by this component.
    pub properties: Vec<ComponentProperty>,
}

/// A configurable property of a component.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComponentProperty {
    /// Property name.
    pub name: String,

    /// Property type (string, number, color, boolean, enum).
    pub kind: PropertyKind,

    /// Default value as a JSON value.
    pub default: Option<serde_json::Value>,

    /// Description.
    pub description: Option<String>,
}

/// Property value types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PropertyKind {
    String,
    Number,
    Boolean,
    Color,
    Enum,
    Asset,
}

/// A style theme — a set of CSS variables / design tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThemeEntry {
    /// Theme identifier.
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Path to the theme data file inside the archive.
    pub path: String,

    /// Whether this is a dark-mode theme.
    pub is_dark: bool,

    /// Description.
    pub description: Option<String>,
}

/// An animation preset that can be applied to any node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AnimationPresetEntry {
    /// Preset identifier.
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Category (e.g., "entrance", "exit", "emphasis", "transition").
    pub category: Option<String>,

    /// Path to the animation data file.
    pub path: String,

    /// Duration in milliseconds.
    pub duration_ms: Option<u32>,

    /// Description.
    pub description: Option<String>,
}

/// An asset bundled with the library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryAsset {
    /// Asset identifier.
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Path inside the archive.
    pub path: String,

    /// MIME type.
    pub mime: Option<String>,

    /// Size in bytes.
    pub size: Option<u64>,

    /// SHA-256 hash for verification.
    pub hash: Option<String>,
}

/// A dependency on another .cxrl library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryDependency {
    /// Library identifier.
    pub id: String,

    /// Required version (semver range like ">=1.0.0").
    pub version: String,

    /// Whether this dependency is optional.
    pub optional: bool,
}

impl LibraryManifest {
    /// Create a new empty library manifest.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            format_version: CXRL_FORMAT_VERSION,
            metadata: LibraryMeta {
                id: id.into(),
                name: name.into(),
                version: version.into(),
                author: None,
                description: None,
                license: None,
                min_runtime_version: None,
                tags: Vec::new(),
            },
            components: Vec::new(),
            themes: Vec::new(),
            animations: Vec::new(),
            assets: Vec::new(),
            dependencies: Vec::new(),
        }
    }
}
