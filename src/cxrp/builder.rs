// openrender-runtime/src/cxrp/builder.rs
//
// Package builder — creates .cxrp archives from documents and assets.

use anyhow::Result;
use std::io::{Write, Cursor};
use std::path::Path;

use crate::cxrd::document::CxrdDocument;
use crate::cxrp::manifest::{
    PackageManifest, DocumentEntry, AssetEntry,
};

/// Builder for constructing .cxrp packages.
pub struct PackageBuilder {
    manifest: PackageManifest,
    /// Raw files to include: (archive_path, data).
    files: Vec<(String, Vec<u8>)>,
}

impl PackageBuilder {
    /// Create a new package builder.
    pub fn new(id: impl Into<String>, name: impl Into<String>, version: impl Into<String>) -> Self {
        Self {
            manifest: PackageManifest::new(id, name, version),
            files: Vec::new(),
        }
    }

    /// Set the package author.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.manifest.metadata.author = Some(author.into());
        self
    }

    /// Set the package description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.manifest.metadata.description = Some(desc.into());
        self
    }

    /// Set the license.
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.manifest.metadata.license = Some(license.into());
        self
    }

    /// Add tags.
    pub fn tags(mut self, tags: Vec<String>) -> Self {
        self.manifest.metadata.tags = tags;
        self
    }

    /// Add a .cxrd document to the package.
    pub fn add_document(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        scene_type: impl Into<String>,
        doc: &CxrdDocument,
    ) -> Result<Self> {
        let id = id.into();
        let archive_path = format!("documents/{}.cxrd", id);

        let data = doc.to_binary()?;

        self.manifest.documents.push(DocumentEntry {
            id: id.clone(),
            name: name.into(),
            path: archive_path.clone(),
            scene_type: scene_type.into(),
            width: Some(doc.viewport_width as u32),
            height: Some(doc.viewport_height as u32),
            hints: None,
        });

        self.files.push((archive_path, data));
        Ok(self)
    }

    /// Add a raw asset file.
    pub fn add_asset(
        mut self,
        name: impl Into<String>,
        archive_path: impl Into<String>,
        mime: Option<String>,
        data: Vec<u8>,
    ) -> Self {
        let path = archive_path.into();
        let name = name.into();
        let size = data.len() as u64;

        // Compute SHA-256 hash.
        use sha2::{Sha256, Digest};
        let hash = hex::encode(Sha256::digest(&data));

        // Categorise by MIME or path.
        let entry = AssetEntry {
            name: name.clone(),
            path: path.clone(),
            mime: mime.clone(),
            size: Some(size),
            hash: Some(hash),
        };

        let mime_lower = mime.as_deref().unwrap_or("").to_lowercase();
        if mime_lower.starts_with("image/") || path.starts_with("assets/images/") {
            self.manifest.assets.images.push(entry);
        } else if mime_lower.starts_with("font/") || path.starts_with("assets/fonts/") {
            self.manifest.assets.fonts.push(entry);
        } else if mime_lower.starts_with("audio/") || path.starts_with("assets/audio/") {
            self.manifest.assets.audio.push(entry);
        } else if mime_lower.starts_with("video/") || path.starts_with("assets/video/") {
            self.manifest.assets.video.push(entry);
        } else {
            self.manifest.assets.data.push(entry);
        }

        self.files.push((path, data));
        self
    }

    /// Add a preview image.
    pub fn add_preview(mut self, archive_path: impl Into<String>, data: Vec<u8>) -> Self {
        let path = archive_path.into();
        self.manifest.metadata.previews.push(path.clone());
        self.files.push((path, data));
        self
    }

    /// Build the .cxrp archive and return it as bytes.
    pub fn build(self) -> Result<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);

            // Write manifest first.
            let manifest_json = serde_json::to_string_pretty(&self.manifest)?;
            zip.start_file::<String, ()>("manifest.json".into(), Default::default())?;
            zip.write_all(manifest_json.as_bytes())?;

            // Write all files.
            for (path, data) in &self.files {
                zip.start_file::<String, ()>(path.clone(), Default::default())?;
                zip.write_all(data)?;
            }

            zip.finish()?;
        }

        Ok(buf.into_inner())
    }

    /// Build and write directly to a file.
    pub fn build_to_file(self, path: impl AsRef<Path>) -> Result<()> {
        let data = self.build()?;
        std::fs::write(path, data)?;
        Ok(())
    }
}
