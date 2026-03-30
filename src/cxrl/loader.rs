// openrender-runtime/src/cxrl/loader.rs
//
// Library loader — reads .cxrl archives and provides access to components,
// themes, animations, and assets.

use anyhow::{Result, Context};
use std::collections::HashMap;
use std::io::{Read, Cursor};
use std::path::Path;

use crate::cxrl::manifest::LibraryManifest;

/// A loaded OpenRender Runtime Library.
pub struct LoadedLibrary {
    /// The parsed manifest.
    pub manifest: LibraryManifest,

    /// Component data by ID: raw bytes of the serialised component subtree.
    components: HashMap<String, Vec<u8>>,

    /// Theme data by ID: raw bytes (typically JSON of variable mappings).
    themes: HashMap<String, Vec<u8>>,

    /// Animation preset data by ID.
    animations: HashMap<String, Vec<u8>>,

    /// Asset data by path.
    assets: HashMap<String, Vec<u8>>,
}

impl LoadedLibrary {
    /// Load a .cxrl library from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let data = std::fs::read(path.as_ref())
            .with_context(|| format!("Failed to read library: {:?}", path.as_ref()))?;
        Self::from_bytes(&data)
    }

    /// Load a .cxrl library from raw bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)
            .context("Failed to open .cxrl archive")?;

        // Read manifest.
        let manifest: LibraryManifest = {
            let mut entry = archive.by_name("manifest.json")
                .context("Missing manifest.json in .cxrl archive")?;
            let mut buf = Vec::new();
            entry.read_to_end(&mut buf)?;
            serde_json::from_slice(&buf)
                .context("Failed to parse library manifest")?
        };

        // Load components.
        let mut components = HashMap::new();
        for comp in &manifest.components {
            if let Ok(mut entry) = archive.by_name(&comp.path) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                components.insert(comp.id.clone(), buf);
            }
        }

        // Load themes.
        let mut themes = HashMap::new();
        for theme in &manifest.themes {
            if let Ok(mut entry) = archive.by_name(&theme.path) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                themes.insert(theme.id.clone(), buf);
            }
        }

        // Load animation presets.
        let mut animations = HashMap::new();
        for anim in &manifest.animations {
            if let Ok(mut entry) = archive.by_name(&anim.path) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                animations.insert(anim.id.clone(), buf);
            }
        }

        // Load assets.
        let mut assets = HashMap::new();
        for asset in &manifest.assets {
            if let Ok(mut entry) = archive.by_name(&asset.path) {
                let mut buf = Vec::new();
                entry.read_to_end(&mut buf)?;
                assets.insert(asset.path.clone(), buf);
            }
        }

        Ok(Self {
            manifest,
            components,
            themes,
            animations,
            assets,
        })
    }

    /// Get component data by ID.
    pub fn get_component(&self, id: &str) -> Option<&[u8]> {
        self.components.get(id).map(|v| v.as_slice())
    }

    /// Get theme data by ID.
    pub fn get_theme(&self, id: &str) -> Option<&[u8]> {
        self.themes.get(id).map(|v| v.as_slice())
    }

    /// Get animation preset data by ID.
    pub fn get_animation(&self, id: &str) -> Option<&[u8]> {
        self.animations.get(id).map(|v| v.as_slice())
    }

    /// Get asset data by archive path.
    pub fn get_asset(&self, path: &str) -> Option<&[u8]> {
        self.assets.get(path).map(|v| v.as_slice())
    }

    /// List all component IDs.
    pub fn component_ids(&self) -> Vec<&str> {
        self.manifest.components.iter().map(|c| c.id.as_str()).collect()
    }

    /// List all theme IDs.
    pub fn theme_ids(&self) -> Vec<&str> {
        self.manifest.themes.iter().map(|t| t.id.as_str()).collect()
    }

    /// List all animation preset IDs.
    pub fn animation_ids(&self) -> Vec<&str> {
        self.manifest.animations.iter().map(|a| a.id.as_str()).collect()
    }

    /// Get the library identifier.
    pub fn id(&self) -> &str {
        &self.manifest.metadata.id
    }

    /// Get the library version.
    pub fn version(&self) -> &str {
        &self.manifest.metadata.version
    }
}
