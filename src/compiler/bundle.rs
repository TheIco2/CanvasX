// canvasx-runtime/src/compiler/bundle.rs
//
// Asset bundler — collects and bundles all local resources (images, fonts)
// referenced by an HTML/CSS scene into the CXRD asset table.

use std::path::Path;
use image::GenericImageView;
use std::fs;
use anyhow::Result;
use crate::cxrd::document::CxrdDocument;

/// Supported image extensions.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "svg"];

/// Supported font extensions.
const FONT_EXTS: &[&str] = &["ttf", "otf", "woff", "woff2"];

/// Bundle all assets from a directory into a CXRD document.
///
/// Scans `asset_dir` for images and fonts, adds them to the document's
/// asset bundle, and returns a mapping of original paths → asset indexes.
pub fn bundle_assets(
    doc: &mut CxrdDocument,
    asset_dir: &Path,
) -> Result<std::collections::HashMap<String, u32>> {
    let mut path_to_index = std::collections::HashMap::new();

    if !asset_dir.exists() {
        return Ok(path_to_index);
    }

    for entry in walkdir::WalkDir::new(asset_dir)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        let ext = path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_lowercase())
            .unwrap_or_default();

        let relative = path.strip_prefix(asset_dir)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");

        if IMAGE_EXTS.contains(&ext.as_str()) {
            let data = fs::read(path)?;
            let (width, height) = get_image_dimensions(&data).unwrap_or((0, 0));
            let mime = match ext.as_str() {
                "png" => "image/png",
                "jpg" | "jpeg" => "image/jpeg",
                "webp" => "image/webp",
                "svg" => "image/svg+xml",
                _ => "application/octet-stream",
            };
            let idx = doc.assets.add_image(
                relative.clone(),
                mime.to_string(),
                data,
                width,
                height,
            );
            path_to_index.insert(relative, idx);
        } else if FONT_EXTS.contains(&ext.as_str()) {
            let data = fs::read(path)?;
            // Extract family name from filename (simplified).
            let family = path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Unknown")
                .to_string();
            let italic = family.to_lowercase().contains("italic");
            let weight = guess_weight_from_name(&family);
            let idx = doc.assets.add_font(family, weight, italic, data);
            path_to_index.insert(relative, idx);
        }
    }

    Ok(path_to_index)
}

/// Try to get image dimensions without fully decoding.
fn get_image_dimensions(data: &[u8]) -> Option<(u32, u32)> {
    image::load_from_memory(data)
        .ok()
        .map(|img| img.dimensions())
}

/// Guess font weight from filename.
fn guess_weight_from_name(name: &str) -> u16 {
    let lower = name.to_lowercase();
    if lower.contains("thin") || lower.contains("hairline") { 100 }
    else if lower.contains("extralight") || lower.contains("ultralight") { 200 }
    else if lower.contains("light") { 300 }
    else if lower.contains("medium") { 500 }
    else if lower.contains("semibold") || lower.contains("demibold") { 600 }
    else if lower.contains("extrabold") || lower.contains("ultrabold") { 800 }
    else if lower.contains("bold") { 700 }
    else if lower.contains("black") || lower.contains("heavy") { 900 }
    else { 400 }
}
