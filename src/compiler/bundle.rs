// prism-runtime/src/compiler/bundle.rs
//
// Asset bundler — collects and bundles all local resources (images, fonts)
// referenced by an HTML/CSS scene into the PRD asset table.

use std::path::Path;
use image::GenericImageView;
use std::fs;
use anyhow::Result;
use crate::prd::document::PrdDocument;

/// Supported image extensions.
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "webp", "svg", "ico"];

/// Supported font extensions.
const FONT_EXTS: &[&str] = &["ttf", "otf", "woff", "woff2"];

/// Bundle all assets from a directory into a PRD document.
///
/// Scans `asset_dir` for images and fonts, adds them to the document's
/// asset bundle, and returns a mapping of original paths → asset indexes.
pub fn bundle_assets(
    doc: &mut PrdDocument,
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

            // .ico files: decode and re-encode as PNG for GPU consumption.
            let (data, mime, width, height) = if ext == "ico" {
                match image::load_from_memory(&data) {
                    Ok(img) => {
                        let (w, h) = img.dimensions();
                        let mut png_bytes = Vec::new();
                        img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;
                        (png_bytes, "image/png", w, h)
                    }
                    Err(e) => {
                        log::warn!("[BUNDLE] Failed to decode .ico '{}': {}", relative, e);
                        continue;
                    }
                }
            } else {
                let (w, h) = get_image_dimensions(&data).unwrap_or((0, 0));
                let m = match ext.as_str() {
                    "png" => "image/png",
                    "jpg" | "jpeg" => "image/jpeg",
                    "webp" => "image/webp",
                    "svg" => "image/svg+xml",
                    _ => "application/octet-stream",
                };
                (data, m, w, h)
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

/// Load a single image file into the document's asset bundle.
/// Returns the asset index on success. Supports png, jpg, webp, svg, ico.
pub fn load_image_asset(doc: &mut PrdDocument, path: &Path) -> Result<u32> {
    let ext = path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    let name = path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("unknown")
        .to_string();

    let raw = fs::read(path)?;

    let (data, mime, width, height) = if ext == "ico" {
        let img = image::load_from_memory(&raw)?;
        let (w, h) = img.dimensions();
        let mut png_bytes = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png_bytes), image::ImageFormat::Png)?;
        (png_bytes, "image/png".to_string(), w, h)
    } else {
        let (w, h) = get_image_dimensions(&raw).unwrap_or((0, 0));
        let m = match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        };
        (raw, m.to_string(), w, h)
    };

    let idx = doc.assets.add_image(name, mime, data, width, height);
    Ok(idx)
}

/// Resolve `<img src="...">` nodes: match `src` attribute against the asset
/// path→index map and update `NodeKind::Image { asset_index }` and
/// `style.background` so the paint system renders the texture.
pub fn resolve_image_nodes(doc: &mut PrdDocument, path_to_index: &std::collections::HashMap<String, u32>) {
    use crate::prd::node::NodeKind;
    use crate::prd::style::Background;
    for node in &mut doc.nodes {
        if let NodeKind::Image { ref mut asset_index, .. } = node.kind {
            if let Some(src) = node.attributes.get("src") {
                // Try exact match, then with forward slashes normalized.
                let normalized = src.replace('\\', "/");
                if let Some(&idx) = path_to_index.get(&normalized)
                    .or_else(|| path_to_index.get(src.as_str()))
                {
                    *asset_index = idx;
                    // Wire into style.background so paint_node emits a textured quad.
                    node.style.background = Background::Image { asset_index: idx };
                }
            }
        }
    }
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

