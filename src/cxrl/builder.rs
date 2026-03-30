// openrender-runtime/src/cxrl/builder.rs
//
// Library builder — creates .cxrl archives from components, themes,
// animations, and assets.

use anyhow::Result;
use std::io::{Write, Cursor};
use std::path::Path;

use crate::cxrl::manifest::{
    LibraryManifest, ComponentEntry, ComponentProperty,
    ThemeEntry, AnimationPresetEntry, LibraryAsset, LibraryDependency,
};

/// Builder for constructing .cxrl library archives.
pub struct LibraryBuilder {
    manifest: LibraryManifest,
    /// Files to include in the archive: (archive_path, data).
    files: Vec<(String, Vec<u8>)>,
}

impl LibraryBuilder {
    /// Create a new library builder.
    pub fn new(
        id: impl Into<String>,
        name: impl Into<String>,
        version: impl Into<String>,
    ) -> Self {
        Self {
            manifest: LibraryManifest::new(id, name, version),
            files: Vec::new(),
        }
    }

    /// Set the library author.
    pub fn author(mut self, author: impl Into<String>) -> Self {
        self.manifest.metadata.author = Some(author.into());
        self
    }

    /// Set the library description.
    pub fn description(mut self, desc: impl Into<String>) -> Self {
        self.manifest.metadata.description = Some(desc.into());
        self
    }

    /// Set the license.
    pub fn license(mut self, license: impl Into<String>) -> Self {
        self.manifest.metadata.license = Some(license.into());
        self
    }

    /// Add a component to the library.
    pub fn add_component(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        data: Vec<u8>,
        properties: Vec<ComponentProperty>,
    ) -> Self {
        let id = id.into();
        let path = format!("components/{}.json", id);

        self.manifest.components.push(ComponentEntry {
            id: id.clone(),
            name: name.into(),
            category: None,
            description: None,
            path: path.clone(),
            properties,
        });

        self.files.push((path, data));
        self
    }

    /// Add a theme to the library.
    pub fn add_theme(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        is_dark: bool,
        data: Vec<u8>,
    ) -> Self {
        let id = id.into();
        let path = format!("themes/{}.json", id);

        self.manifest.themes.push(ThemeEntry {
            id: id.clone(),
            name: name.into(),
            path: path.clone(),
            is_dark,
            description: None,
        });

        self.files.push((path, data));
        self
    }

    /// Add an animation preset.
    pub fn add_animation(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        category: Option<String>,
        duration_ms: Option<u32>,
        data: Vec<u8>,
    ) -> Self {
        let id = id.into();
        let path = format!("animations/{}.json", id);

        self.manifest.animations.push(AnimationPresetEntry {
            id: id.clone(),
            name: name.into(),
            category,
            path: path.clone(),
            duration_ms,
            description: None,
        });

        self.files.push((path, data));
        self
    }

    /// Add an asset to the library.
    pub fn add_asset(
        mut self,
        id: impl Into<String>,
        name: impl Into<String>,
        mime: Option<String>,
        data: Vec<u8>,
    ) -> Self {
        let id_str = id.into();
        let ext = mime.as_deref().and_then(mime_to_ext).unwrap_or("bin");
        let path = format!("assets/{}.{}", id_str, ext);
        let size = data.len() as u64;

        use sha2::{Sha256, Digest};
        let hash = hex::encode(Sha256::digest(&data));

        self.manifest.assets.push(LibraryAsset {
            id: id_str,
            name: name.into(),
            path: path.clone(),
            mime,
            size: Some(size),
            hash: Some(hash),
        });

        self.files.push((path, data));
        self
    }

    /// Add a dependency on another library.
    pub fn add_dependency(
        mut self,
        id: impl Into<String>,
        version: impl Into<String>,
        optional: bool,
    ) -> Self {
        self.manifest.dependencies.push(LibraryDependency {
            id: id.into(),
            version: version.into(),
            optional,
        });
        self
    }

    /// Build the .cxrl archive and return it as bytes.
    pub fn build(self) -> Result<Vec<u8>> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);

            // Write manifest.
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

/// Helper: map a MIME type to a file extension.
fn mime_to_ext(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" | "image/jpg" => Some("jpg"),
        "image/svg+xml" => Some("svg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "font/ttf" | "font/truetype" => Some("ttf"),
        "font/otf" | "font/opentype" => Some("otf"),
        "font/woff" => Some("woff"),
        "font/woff2" => Some("woff2"),
        "audio/mpeg" | "audio/mp3" => Some("mp3"),
        "audio/ogg" => Some("ogg"),
        "audio/wav" => Some("wav"),
        "video/mp4" => Some("mp4"),
        "video/webm" => Some("webm"),
        "application/json" => Some("json"),
        "text/css" => Some("css"),
        "text/javascript" | "application/javascript" => Some("js"),
        _ => None,
    }
}
