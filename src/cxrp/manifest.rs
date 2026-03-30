// openrender-runtime/src/cxrp/manifest.rs
//
// Package manifest — describes the contents and metadata of a .cxrp archive.

use serde::{Serialize, Deserialize};
use std::collections::HashMap;

/// The manifest for a OpenRender Runtime Package (.cxrp).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageManifest {
    /// Package format version.
    pub format_version: u32,

    /// Package metadata.
    pub metadata: PackageMeta,

    /// List of documents (.cxrd) in this package.
    pub documents: Vec<DocumentEntry>,

    /// Shared asset declarations (path relative to archive root).
    pub assets: AssetManifest,

    /// Referenced libraries (.cxrl).
    pub libraries: Vec<LibraryRef>,

    /// Dependency relationships (document → library mappings).
    pub dependencies: HashMap<String, Vec<String>>,
}

/// Package-level metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageMeta {
    /// Unique package identifier (e.g., "com.example.my-wallpaper-pack").
    pub id: String,

    /// Human-readable name.
    pub name: String,

    /// Semantic version string.
    pub version: String,

    /// Author name or organisation.
    pub author: Option<String>,

    /// Author URL.
    pub author_url: Option<String>,

    /// Short description (one line).
    pub description: Option<String>,

    /// Long description (Markdown).
    pub long_description: Option<String>,

    /// License identifier (e.g., "MIT", "CC-BY-4.0").
    pub license: Option<String>,

    /// Tags for discovery.
    pub tags: Vec<String>,

    /// Minimum OpenRender runtime version required.
    pub min_runtime_version: Option<String>,

    /// Preview image paths (relative to archive root).
    pub previews: Vec<String>,
}

/// An entry describing a single .cxrd document in the package.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentEntry {
    /// Unique document ID within this package.
    pub id: String,

    /// Display name.
    pub name: String,

    /// Path within the archive (e.g., "documents/clock-widget.cxrd").
    pub path: String,

    /// What kind of document this is.
    pub scene_type: String, // "wallpaper", "widget", "statusbar", etc.

    /// Default dimensions (design resolution).
    pub width: Option<u32>,
    pub height: Option<u32>,

    /// Performance hints.
    pub hints: Option<PerformanceHints>,
}

/// Performance hints for a document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PerformanceHints {
    /// Preferred frame rate (0 = match display).
    pub target_fps: Option<u32>,

    /// GPU usage level: "low", "medium", "high".
    pub gpu_usage: Option<String>,

    /// Number of compositing layers expected.
    pub layer_count: Option<u32>,
}

/// Asset inventory for the package.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetManifest {
    /// Image files.
    pub images: Vec<AssetEntry>,

    /// Font files.
    pub fonts: Vec<AssetEntry>,

    /// Audio files.
    pub audio: Vec<AssetEntry>,

    /// Video files.
    pub video: Vec<AssetEntry>,

    /// Other data files.
    pub data: Vec<AssetEntry>,
}

/// An asset file entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AssetEntry {
    /// Asset name/identifier.
    pub name: String,

    /// Path within the archive.
    pub path: String,

    /// MIME type.
    pub mime: Option<String>,

    /// File size in bytes.
    pub size: Option<u64>,

    /// SHA-256 hash for integrity verification.
    pub hash: Option<String>,
}

/// A reference to a .cxrl library.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryRef {
    /// Library identifier.
    pub id: String,

    /// Version constraint (semver).
    pub version: String,

    /// Path within the archive (if bundled), or None if external.
    pub path: Option<String>,
}

impl PackageManifest {
    /// Create a new empty package manifest.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            format_version: 1,
            metadata: PackageMeta {
                id: id.into(),
                name: name.into(),
                version: version.into(),
                author: None,
                author_url: None,
                description: None,
                long_description: None,
                license: None,
                tags: Vec::new(),
                min_runtime_version: None,
                previews: Vec::new(),
            },
            documents: Vec::new(),
            assets: AssetManifest::default(),
            libraries: Vec::new(),
            dependencies: HashMap::new(),
        }
    }
}
