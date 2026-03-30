// openrender-runtime/src/cxrd/asset.rs
//
// Asset table for bundled resources within a CXRD document.
// All assets are embedded — no network fetches at runtime.

use serde::{Serialize, Deserialize};

/// The complete asset bundle for a CXRD document.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AssetBundle {
    /// Image assets (textures).
    pub images: Vec<ImageAsset>,

    /// Font assets.
    pub fonts: Vec<FontAsset>,

    /// Raw data blobs (e.g., shader includes, JSON data files).
    pub data: Vec<DataAsset>,
}

/// A bundled image asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ImageAsset {
    /// Original filename (for debugging).
    pub name: String,

    /// MIME type (image/png, image/jpeg, image/webp).
    pub mime: String,

    /// Raw image bytes (decoded at load time, not every frame).
    #[serde(with = "serde_bytes_compat")]
    pub data: Vec<u8>,

    /// Image dimensions (if known at compile time).
    pub width: u32,
    pub height: u32,
}

/// A bundled font asset.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FontAsset {
    /// Font family name.
    pub family: String,

    /// Font weight (100–900).
    pub weight: u16,

    /// Italic flag.
    pub italic: bool,

    /// Raw font file bytes (TTF/OTF/WOFF2).
    #[serde(with = "serde_bytes_compat")]
    pub data: Vec<u8>,
}

/// A bundled data blob.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataAsset {
    pub name: String,
    pub mime: String,
    #[serde(with = "serde_bytes_compat")]
    pub data: Vec<u8>,
}

impl AssetBundle {
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an image asset, returning its index.
    pub fn add_image(&mut self, name: String, mime: String, data: Vec<u8>, width: u32, height: u32) -> u32 {
        let idx = self.images.len() as u32;
        self.images.push(ImageAsset { name, mime, data, width, height });
        idx
    }

    /// Add a font asset, returning its index.
    pub fn add_font(&mut self, family: String, weight: u16, italic: bool, data: Vec<u8>) -> u32 {
        let idx = self.fonts.len() as u32;
        self.fonts.push(FontAsset { family, weight, italic, data });
        idx
    }
}

/// Serde helper for Vec<u8> that uses base64 in JSON but raw bytes in bincode.
mod serde_bytes_compat {
    use serde::{Serializer, Deserializer, Serialize, Deserialize};

    pub fn serialize<S: Serializer>(bytes: &Vec<u8>, s: S) -> Result<S::Ok, S::Error> {
        bytes.serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        Vec::<u8>::deserialize(d)
    }
}
