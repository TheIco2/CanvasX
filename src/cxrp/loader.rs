// openrender-runtime/src/cxrp/loader.rs
//
// Package loader — reads .cxrp archives and extracts documents and assets.
// A .cxrp is a ZIP archive with a manifest.json at the root.

use anyhow::{Result, Context};
use std::io::{Read, Cursor};
use std::path::Path;
use std::collections::HashMap;

use crate::cxrd::document::CxrdDocument;
use crate::cxrp::manifest::PackageManifest;

/// A loaded OpenRender Runtime Package.
pub struct LoadedPackage {
    /// The package manifest.
    pub manifest: PackageManifest,

    /// Loaded documents, keyed by document ID.
    pub documents: HashMap<String, CxrdDocument>,

    /// Raw asset data, keyed by archive path.
    pub assets: HashMap<String, Vec<u8>>,
}

impl LoadedPackage {
    /// Load a .cxrp package from a file path.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self> {
        let data = std::fs::read(path.as_ref())
            .with_context(|| format!("Failed to read package: {}", path.as_ref().display()))?;
        Self::from_bytes(&data)
    }

    /// Load a .cxrp package from bytes.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let cursor = Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)
            .context("Failed to open .cxrp archive (invalid ZIP)")?;

        // Read manifest.
        let manifest: PackageManifest = {
            let mut manifest_file = archive
                .by_name("manifest.json")
                .context("Package missing manifest.json")?;
            let mut buf = String::new();
            manifest_file.read_to_string(&mut buf)?;
            serde_json::from_str(&buf)
                .context("Invalid manifest.json")?
        };

        // Load assets.
        let mut assets: HashMap<String, Vec<u8>> = HashMap::new();
        for i in 0..archive.len() {
            let mut file = archive.by_index(i)?;
            let name = file.name().to_string();

            if name == "manifest.json" {
                continue;
            }

            // Read asset data.
            let mut buf = Vec::new();
            file.read_to_end(&mut buf)?;
            assets.insert(name, buf);
        }

        // Load .cxrd documents.
        let mut documents = HashMap::new();
        for entry in &manifest.documents {
            if let Some(data) = assets.get(&entry.path) {
                match CxrdDocument::from_binary(data) {
                    Ok(doc) => {
                        documents.insert(entry.id.clone(), doc);
                    }
                    Err(e) => {
                        log::warn!("Failed to load document '{}': {}", entry.id, e);
                    }
                }
            } else {
                log::warn!("Document '{}' listed in manifest but not found in archive", entry.id);
            }
        }

        Ok(Self {
            manifest,
            documents,
            assets,
        })
    }

    /// Get a specific document by ID.
    pub fn get_document(&self, id: &str) -> Option<&CxrdDocument> {
        self.documents.get(id)
    }

    /// Get raw asset data by archive path.
    pub fn get_asset(&self, path: &str) -> Option<&[u8]> {
        self.assets.get(path).map(|v| v.as_slice())
    }

    /// List all document IDs.
    pub fn document_ids(&self) -> Vec<&str> {
        self.manifest.documents.iter().map(|d| d.id.as_str()).collect()
    }
}
