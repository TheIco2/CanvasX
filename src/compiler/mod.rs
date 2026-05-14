// prism-runtime/src/compiler/mod.rs
//
// Prism HTML/CSS → PRD compiler.
// Converts a restricted subset of HTML+CSS into a compiled PRD document.
// This runs at load time (not per-frame), and the result is cached to disk.

pub mod html;
pub mod css;
pub mod bundle;
pub mod editable;

use std::fs;
use std::path::{Path};
use anyhow::{anyhow, Result};
use regex::Regex;
use crate::prd::document::SceneType;

/// High-level CLI compilation API
/// 
/// Compiles an HTML file with its associated CSS/JS to a .prd file
pub fn compile_html_file(html_path: &Path) -> Result<()> {
    if !html_path.exists() {
        return Err(anyhow!("HTML file not found: {:?}", html_path));
    }

    let html_content = fs::read_to_string(html_path)?;
    
    // Extract metadata from HTML meta tags
    let scene_type = extract_meta_tag(&html_content, "prism:type")
        .and_then(|t| match t.to_lowercase().as_str() {
            "widget" => Some(SceneType::Widget),
            "page" => Some(SceneType::Widget),
            "wallpaper" => Some(SceneType::Wallpaper),
            _ => None,
        })
        .unwrap_or(SceneType::Widget);

    let name = extract_meta_tag(&html_content, "prism:name")
        .unwrap_or_else(|| "Untitled".to_string());

    // Find and read CSS file
    let css_path = html_path.parent()
        .map(|p| p.join(html_path.file_stem().unwrap_or_default()).with_extension("css"));
    
    let css_content = if let Some(css_p) = css_path {
        if css_p.exists() {
            fs::read_to_string(&css_p).unwrap_or_default()
        } else {
            String::new()
        }
    } else {
        String::new()
    };

    // Find referenced stylesheets
    let stylesheet_dir = html_path.parent();
    let mut all_css = css_content.clone();
    
    if let Ok(re) = Regex::new(r#"<link\s+rel=["']stylesheet["']\s+href=["']([^"']*)["']"#) {
        for cap in re.captures_iter(&html_content) {
            if let Some(href) = cap.get(1) {
                if let Some(base) = stylesheet_dir {
                    let css_file = base.join(href.as_str());
                    if css_file.exists() {
                        if let Ok(content) = fs::read_to_string(&css_file) {
                            all_css.push('\n');
                            all_css.push_str(&content);
                        }
                    }
                }
            }
        }
    }

    // Compile using the existing compiler
    let (doc, scripts, _rules) = html::compile_html(
        &html_content,
        &all_css,
        &name,
        scene_type,
        stylesheet_dir,
    )?;

    // Save to .prd file
    let prd_path = html_path.with_extension("prd");
    let prd_data = doc.to_binary()?;
    fs::write(&prd_path, &prd_data)?;

    // Print summary
    println!("[COMPILER] Type: {:?}", scene_type);
    println!("[COMPILER] Name: {}", name);
    println!("[COMPILER] Nodes: {}", doc.nodes.len());
    println!("[COMPILER] Scripts: {}", scripts.len());
    println!("✓ Compiled to: {}", prd_path.display());
    println!("  Size: {} bytes", prd_data.len());

    Ok(())
}

/// Extract metadata from HTML meta tags  
fn extract_meta_tag(html: &str, tag_name: &str) -> Option<String> {
    // Try double quotes first
    if let Ok(re) = Regex::new(&format!(r#"<meta name="{}" content="([^"]*)""#, tag_name)) {
        if let Some(cap) = re.captures(html) {
            if let Some(content) = cap.get(1) {
                return Some(content.as_str().to_string());
            }
        }
    }

    // Try single quotes
    if let Ok(re) = Regex::new(&format!(r#"<meta name='{}' content='([^']*)'"#, tag_name)) {
        if let Some(cap) = re.captures(html) {
            if let Some(content) = cap.get(1) {
                return Some(content.as_str().to_string());
            }
        }
    }

    None
}

