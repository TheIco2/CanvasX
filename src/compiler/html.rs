// prism-runtime/src/compiler/html.rs
//
// HTML subset parser for the OpenRender Runtime.
// Converts restricted HTML into PRD nodes.
//
// Supported elements:
//   div, span, p, h1–h6, img, button, input, label, svg, path, section
//   Custom: <data-bind> for live system data.
//
// Attributes: class, id, style (inline), data-*, src, alt

use crate::prd::document::{PrdDocument, SceneType};
use crate::prd::node::{PrdNode, NodeKind, ImageFit, NodeId, EventBinding, EventAction};
use crate::prd::input::{InputKind, TextInputType, ButtonVariant, CheckboxStyle};
use crate::prd::style::{AlignItems, Background, ComputedStyle, CursorStyle, Display, FlexDirection, FontWeight, TextAlign};
use crate::prd::value::{Color, Dimension};
use crate::compiler::css::{parse_css, apply_property, parse_color, CssRule, CompoundSelector};
use std::collections::HashMap;
use std::path::Path;

/// A collected script block from the HTML source.
#[derive(Debug, Clone)]
pub struct ScriptBlock {
    /// Inline script text content, or empty if external.
    pub content: String,
    /// External script src path (relative to HTML file).
    pub src: Option<String>,
    /// Whether this script should run after all other scripts (deferred loading).
    pub deferred: bool,
}

// Thread-local storage for collecting scripts during tokenization.
std::thread_local! {
    static COLLECTED_SCRIPTS: std::cell::RefCell<Vec<ScriptBlock>> = std::cell::RefCell::new(Vec::new());
}

/// Preprocess HTML to inline native asset references and component includes.
///
/// Supported forms:
///   `<link rel="stylesheet" href="path">`             — inlined as `<style>…</style>`
///   `<link rel="icon"|"shortcut icon" href="path">`   — emitted as `<meta name="icon">`
///   `<script src="path"></script>`                    — inlined at position
///   `<script src="path" defer></script>`              — appended at end of document
///   `<include type="component" src="name" />`         — recursively inlined as HTML
///   `<include src="name" />`                          — alias for component include
///
/// `<style>`, inline `<script>`, and other `<link>` rels are passed through
/// unchanged. Recursion depth is capped at 16.
fn preprocess_includes(html: &str, base_dir: Option<&Path>, depth: u32) -> String {
    let (mut result, deferred) = preprocess_includes_inner(html, base_dir, depth);
    for script in &deferred {
        result.push_str("\n<script data-deferred=\"true\">\n");
        result.push_str(script);
        result.push_str("\n</script>");
    }
    result
}

fn preprocess_includes_inner(html: &str, base_dir: Option<&Path>, depth: u32) -> (String, Vec<String>) {
    preprocess_includes_inner_with_embed(html, base_dir, None, depth)
}

/// Embed-aware preprocessor — same rules as [`preprocess_includes`] but
/// resolves `href`/`src` paths against the in-memory `pages/` bundle via
/// [`crate::embed::read_page_str`] instead of the filesystem.
///
/// `embed_base` is a relative path within the embedded bundle that acts as
/// the resolution root (e.g. `""` for pages at the bundle root, or
/// `"settings"` for pages under `pages/settings/`).
pub fn preprocess_includes_embedded(html: &str, embed_base: &str, depth: u32) -> String {
    let (mut result, deferred) =
        preprocess_includes_inner_with_embed(html, None, Some(embed_base), depth);
    for script in &deferred {
        result.push_str("\n<script data-deferred=\"true\">\n");
        result.push_str(script);
        result.push_str("\n</script>");
    }
    result
}

/// Read an asset (relative path) from either the embedded bundle or filesystem.
fn read_asset(src: &str, base_dir: Option<&Path>, embed_base: Option<&str>) -> Option<String> {
    let src = src.trim();
    if let Some(eb) = embed_base {
        let rel = join_embed(eb, "", src);
        crate::embed::read_page_str(&rel).map(|s| s.to_string())
    } else {
        let path = match base_dir {
            Some(b) => b.join(src),
            None => Path::new(src).to_path_buf(),
        };
        std::fs::read_to_string(&path).ok()
    }
}

/// Compute a display path string for icon metadata (no file read).
fn resolve_asset_path(src: &str, base_dir: Option<&Path>, embed_base: Option<&str>) -> String {
    let src = src.trim();
    if let Some(eb) = embed_base {
        join_embed(eb, "", src)
    } else {
        match base_dir {
            Some(b) => b.join(src).to_string_lossy().replace('\\', "/"),
            None => src.replace('\\', "/"),
        }
    }
}

fn preprocess_includes_inner_with_embed(
    html: &str,
    base_dir: Option<&Path>,
    embed_base: Option<&str>,
    depth: u32,
) -> (String, Vec<String>) {
    if depth > 16 {
        log::warn!("include depth limit reached (>16), stopping recursion");
        return (html.to_string(), Vec::new());
    }

    let mut result = String::with_capacity(html.len());
    let mut deferred_scripts: Vec<String> = Vec::new();
    let lower = html.to_lowercase();
    let bytes = html.as_bytes();
    let mut pos = 0;

    // Find the next occurrence of any of the recognised tag prefixes.
    // The character after the prefix must be whitespace, `>`, or `/` to avoid
    // matching custom tags like `<link-preview>` or `<scripted>`.
    fn next_tag(lower: &str, pos: usize) -> Option<(usize, &'static str)> {
        let candidates: &[&'static str] = &["<include", "<link", "<script", "<style"];
        let bytes = lower.as_bytes();
        let mut best: Option<(usize, &'static str)> = None;
        for &needle in candidates {
            let mut search = pos;
            while let Some(i) = lower[search..].find(needle) {
                let abs = search + i;
                let next_byte = bytes.get(abs + needle.len()).copied().unwrap_or(b' ');
                if next_byte == b' ' || next_byte == b'\t' || next_byte == b'\n'
                    || next_byte == b'\r' || next_byte == b'>' || next_byte == b'/'
                {
                    if best.map(|(b, _)| abs < b).unwrap_or(true) {
                        best = Some((abs, needle));
                    }
                    break;
                }
                search = abs + needle.len();
            }
        }
        best
    }

    while pos < bytes.len() {
        let (abs, kind) = match next_tag(&lower, pos) {
            Some(t) => t,
            None => {
                result.push_str(&html[pos..]);
                break;
            }
        };
        result.push_str(&html[pos..abs]);

        // Locate the end of the opening tag — the first `>` outside of a
        // quoted attribute value. Using `find("/>")` here is unsafe because
        // it would jump to the next `/>` *anywhere* in the document if this
        // particular tag isn't self-closing.
        let after_tag = abs + kind.len();
        let open_end = match find_tag_end(html, after_tag) {
            Some(e) => e,
            None => {
                result.push_str(&html[abs..after_tag]);
                pos = after_tag;
                continue;
            }
        };
        let open_text = &html[abs..open_end];
        let self_closing = open_text.trim_end_matches('>').trim_end().ends_with('/');

        match kind {
            "<include" => {
                // Find the closing </include> (only matters when not self-closing).
                let tag_end = if self_closing {
                    open_end
                } else if let Some(close) = lower[open_end..].find("</include>") {
                    open_end + close + "</include>".len()
                } else {
                    open_end
                };
                let tag_text = &html[abs..tag_end];
                let src = extract_attribute(tag_text, "src");
                let include_type = extract_attribute(tag_text, "type");

                // Reject legacy types so old templates fail loudly.
                match include_type.as_deref() {
                    Some("asset") | Some("icon") => {
                        log::error!(
                            "include: type=\"{}\" is no longer supported — use <link>/<script> instead ({})",
                            include_type.as_deref().unwrap_or(""), tag_text,
                        );
                        result.push_str(&format!(
                            "<!-- unsupported include type=\"{}\"; use native <link>/<script> -->",
                            include_type.as_deref().unwrap_or(""),
                        ));
                        pos = tag_end;
                        continue;
                    }
                    _ => {}
                }

                let src_path = match src {
                    Some(s) => s,
                    None => {
                        log::warn!("include tag without src attribute: {}", tag_text);
                        pos = tag_end;
                        continue;
                    }
                };

                // Both `type="component"` and bare `<include>` resolve from the
                // `components/` subdirectory.
                if let Some(eb) = embed_base {
                    let resolved_rel = join_embed(eb, "components", &src_path);
                    match crate::embed::read_page_str(&resolved_rel) {
                        Some(contents) => {
                            let child_base = parent_of(&resolved_rel);
                            let (expanded, child_deferred) =
                                preprocess_includes_inner_with_embed(
                                    contents, None, Some(&child_base), depth + 1,
                                );
                            result.push_str(&expanded);
                            deferred_scripts.extend(child_deferred);
                        }
                        None => {
                            log::error!("include: failed to read embedded '{}'", resolved_rel);
                            result.push_str(&format!(
                                "<!-- include error: embedded '{}' not found -->",
                                resolved_rel
                            ));
                        }
                    }
                } else {
                    let resolved = base_dir
                        .map(|b| b.join("components").join(&src_path))
                        .unwrap_or_else(|| Path::new(&src_path).to_path_buf());
                    match std::fs::read_to_string(&resolved) {
                        Ok(contents) => {
                            let child_dir = resolved.parent().or(base_dir);
                            let (expanded, child_deferred) =
                                preprocess_includes_inner_with_embed(
                                    &contents, child_dir, None, depth + 1,
                                );
                            result.push_str(&expanded);
                            deferred_scripts.extend(child_deferred);
                        }
                        Err(e) => {
                            log::error!("include: failed to read '{}': {}", resolved.display(), e);
                            result.push_str(&format!("<!-- include error: {} -->", e));
                        }
                    }
                }
                pos = tag_end;
            }

            "<link" => {
                let rel = extract_attribute(open_text, "rel")
                    .map(|s| s.to_ascii_lowercase())
                    .unwrap_or_default();
                let href = extract_attribute(open_text, "href");

                if rel == "stylesheet" {
                    if let Some(h) = href {
                        match read_asset(&h, base_dir, embed_base) {
                            Some(contents) => {
                                result.push_str("<style>\n");
                                result.push_str(&contents);
                                result.push_str("\n</style>");
                            }
                            None => {
                                log::error!("link: failed to read stylesheet '{}'", h);
                                result.push_str(&format!(
                                    "<!-- stylesheet not found: {} -->", h
                                ));
                            }
                        }
                    }
                } else if rel == "icon" || rel == "shortcut icon" {
                    if let Some(h) = href {
                        let path = resolve_asset_path(&h, base_dir, embed_base);
                        let target = extract_attribute(open_text, "data-target")
                            .unwrap_or_default();
                        result.push_str(&format!(
                            "<meta name=\"icon\" data-target=\"{}\" content=\"{}\" />",
                            target, path
                        ));
                    }
                } else {
                    // Other rels (preconnect, manifest, etc.) — pass through.
                    result.push_str(open_text);
                }
                pos = open_end;
            }

            "<script" => {
                let src = extract_attribute(open_text, "src");
                if let Some(s) = src {
                    // External script — locate the matching </script> so we
                    // consume any (typically empty) body too.
                    let body_end = if self_closing {
                        open_end
                    } else if let Some(close) = lower[open_end..].find("</script>") {
                        open_end + close + "</script>".len()
                    } else {
                        open_end
                    };
                    let defer = has_attribute(open_text, "defer");
                    match read_asset(&s, base_dir, embed_base) {
                        Some(contents) => {
                            if defer {
                                deferred_scripts.push(contents);
                            } else {
                                result.push_str("<script>\n");
                                result.push_str(&contents);
                                result.push_str("\n</script>");
                            }
                        }
                        None => {
                            log::error!("script: failed to read '{}'", s);
                            result.push_str(&format!("<!-- script not found: {} -->", s));
                        }
                    }
                    pos = body_end;
                } else {
                    // Inline script — copy the entire <script>...</script>
                    // block verbatim so JS contents aren't re-scanned for our
                    // tag prefixes.
                    let body_end = if self_closing {
                        open_end
                    } else if let Some(close) = lower[open_end..].find("</script>") {
                        open_end + close + "</script>".len()
                    } else {
                        open_end
                    };
                    result.push_str(&html[abs..body_end]);
                    pos = body_end;
                }
            }

            "<style" => {
                // Pass through the entire <style>...</style> block verbatim.
                let body_end = if self_closing {
                    open_end
                } else if let Some(close) = lower[open_end..].find("</style>") {
                    open_end + close + "</style>".len()
                } else {
                    open_end
                };
                result.push_str(&html[abs..body_end]);
                pos = body_end;
            }

            _ => unreachable!(),
        }
    }

    (result, deferred_scripts)
}

/// Join `(base, sub, src)` into a normalised forward-slash relative path used
/// for embedded bundle lookups.
///
/// Examples:
///   `join_embed("",          "",           "css/x.css") -> "css/x.css"`
///   `join_embed("settings",  "components", "btn.html")  -> "settings/components/btn.html"`
fn join_embed(base: &str, sub: &str, src: &str) -> String {
    let mut out = String::new();
    let base = base.trim_matches('/');
    let src = src.trim_start_matches('/');
    if !base.is_empty() {
        out.push_str(base);
        out.push('/');
    }
    if !sub.is_empty() {
        out.push_str(sub);
        out.push('/');
    }
    out.push_str(src);
    out.replace('\\', "/")
}

/// Return the parent (directory) portion of an embedded relative path, or "".
fn parent_of(rel: &str) -> String {
    match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    }
}

/// Preprocess `<page-content>` tags by inlining the default page fragment.
///
/// `<page-content default="devices" />` becomes:
/// `<page-content data-default="devices" data-active-content="devices">[devices.html content]</page-content>`
fn preprocess_page_content(html: &str, base_dir: Option<&Path>) -> String {
    preprocess_page_content_with_embed(html, base_dir, None)
}

/// Embed-aware variant — when `embed_base` is `Some`, the default fragment is
/// resolved against the in-memory `pages/` bundle.
pub fn preprocess_page_content_embedded(html: &str, embed_base: &str) -> String {
    preprocess_page_content_with_embed(html, None, Some(embed_base))
}

fn preprocess_page_content_with_embed(
    html: &str,
    base_dir: Option<&Path>,
    embed_base: Option<&str>,
) -> String {
    let mut result = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let bytes = html.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        if let Some(idx) = lower[pos..].find("<page-content") {
            let abs = pos + idx;
            result.push_str(&html[pos..abs]);

            let after_tag = abs + "<page-content".len();
            let open_end = match find_tag_end(html, after_tag) {
                Some(e) => e,
                None => {
                    result.push_str(&html[abs..after_tag]);
                    pos = after_tag;
                    continue;
                }
            };
            let self_closing = html[..open_end]
                .trim_end_matches('>')
                .trim_end()
                .ends_with('/');
            let tag_end = if self_closing {
                open_end
            } else if let Some(close) = lower[open_end..].find("</page-content>") {
                open_end + close + "</page-content>".len()
            } else {
                open_end
            };

            let tag_text = &html[abs..tag_end];
            let default_page = extract_attribute(tag_text, "default");
            // If the tag has already been expanded (carries `data-default`),
            // pass it through verbatim so a second preprocessing pass is a
            // no-op rather than wiping the inlined content.
            let already_expanded = extract_attribute(tag_text, "data-default").is_some()
                || extract_attribute(tag_text, "data-active-content").is_some();

            if already_expanded {
                result.push_str(tag_text);
            } else if let Some(ref default_id) = default_page {
                // Load the default content fragment from either the embedded
                // bundle or filesystem.
                let content = if let Some(eb) = embed_base {
                    let rel = if eb.is_empty() {
                        format!("{}.html", default_id)
                    } else {
                        format!("{}/{}.html", eb.trim_matches('/'), default_id)
                    };
                    crate::embed::read_page_str(&rel)
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| {
                            log::error!("page-content: embedded fragment '{}' not found", rel);
                            String::new()
                        })
                } else {
                    base_dir
                        .map(|b| b.join(format!("{}.html", default_id)))
                        .and_then(|p| std::fs::read_to_string(&p).ok())
                        .unwrap_or_default()
                };
                result.push_str(&format!(
                    "<page-content data-default=\"{}\" data-active-content=\"{}\">\n{}\n</page-content>",
                    default_id, default_id, content
                ));
            } else {
                result.push_str("<page-content></page-content>");
            }

            pos = tag_end;
        } else {
            result.push_str(&html[pos..]);
            break;
        }
    }

    result
}

/// Check whether a tag string contains a boolean attribute (no `=value`).
fn has_attribute(tag: &str, attr_name: &str) -> bool {
    let lower = tag.to_lowercase();
    let name = attr_name.to_lowercase();
    lower.split_whitespace().any(|word| {
        let word = word.trim_end_matches('/').trim_end_matches('>');
        word == name
    })
}

/// Extract the value of `<meta name="redirect" content="...">` if present.
fn extract_redirect(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut search_pos = 0;
    while let Some(idx) = lower[search_pos..].find("<meta") {
        let abs = search_pos + idx;
        let after = abs + 5;
        let tag_end = if let Some(gt) = lower[after..].find('>') {
            after + gt + 1
        } else {
            break;
        };
        let tag_text = &html[abs..tag_end];
        if extract_attribute(tag_text, "name").as_deref() == Some("redirect") {
            return extract_attribute(tag_text, "content");
        }
        search_pos = tag_end;
    }
    None
}

/// Extract icon declarations from `<meta name="icon" ...>` tags
/// (emitted by the native `<link rel="icon">` / `<link rel="shortcut icon">`
/// preprocessing).
fn extract_icons(html: &str) -> Vec<crate::prd::document::IconDecl> {
    let lower = html.to_lowercase();
    let mut icons = Vec::new();
    let mut search_pos = 0;
    while let Some(idx) = lower[search_pos..].find("<meta") {
        let abs = search_pos + idx;
        let after = abs + 5;
        let tag_end = if let Some(gt) = lower[after..].find('>') {
            after + gt + 1
        } else {
            break;
        };
        let tag_text = &html[abs..tag_end];
        if extract_attribute(tag_text, "name").as_deref() == Some("icon") {
            if let Some(path) = extract_attribute(tag_text, "content") {
                let target = extract_attribute(tag_text, "data-target").unwrap_or_default();
                icons.push(crate::prd::document::IconDecl { target, path, asset_index: None });
            }
        }
        search_pos = tag_end;
    }
    icons
}

/// Extract the document title from `<title>…</title>` tags.
/// If multiple `<title>` tags exist, the last one wins.
fn extract_title(html: &str) -> Option<String> {
    let lower = html.to_lowercase();
    let mut title: Option<String> = None;
    let mut search_start = 0;
    while let Some(open) = lower[search_start..].find("<title") {
        let abs_open = search_start + open;
        if let Some(gt) = lower[abs_open..].find('>') {
            let content_start = abs_open + gt + 1;
            if let Some(close) = lower[content_start..].find("</title>") {
                let content_end = content_start + close;
                let t = html[content_start..content_end].trim();
                if !t.is_empty() {
                    title = Some(t.to_string());
                }
                search_start = content_end + "</title>".len();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    title
}

/// Find the byte index just past the closing `>` of an HTML opening tag,
/// starting at `after_tag` (the byte index immediately after the tag name).
/// Skips over `>` characters that appear inside quoted attribute values.
/// Returns `None` if no closing `>` is found.
fn find_tag_end(html: &str, after_tag: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let mut i = after_tag;
    let mut quote: Option<u8> = None;
    while i < bytes.len() {
        let b = bytes[i];
        if let Some(q) = quote {
            if b == q { quote = None; }
        } else if b == b'"' || b == b'\'' {
            quote = Some(b);
        } else if b == b'>' {
            return Some(i + 1);
        }
        i += 1;
    }
    None
}

/// Extract a named attribute value from a tag string.
fn extract_attribute(tag: &str, attr_name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let bytes = lower.as_bytes();
    let needle = format!("{}=", attr_name);
    let mut search = 0;
    while let Some(rel) = lower[search..].find(&needle) {
        let idx = search + rel;
        // Require a word boundary before the attribute name so that
        // `default=` doesn't accidentally match inside `data-default=`,
        // and `src=` doesn't match inside e.g. `data-src=`.
        let prev = if idx == 0 { b' ' } else { bytes[idx - 1] };
        let is_boundary = matches!(
            prev,
            b' ' | b'\t' | b'\n' | b'\r' | b'/' | b'<'
        );
        if !is_boundary {
            search = idx + needle.len();
            continue;
        }
        let after_eq = idx + needle.len();
        let rest = tag[after_eq..].trim_start();
        if rest.starts_with('"') {
            let inner = &rest[1..];
            if let Some(end) = inner.find('"') {
                return Some(inner[..end].to_string());
            }
        } else if rest.starts_with('\'') {
            let inner = &rest[1..];
            if let Some(end) = inner.find('\'') {
                return Some(inner[..end].to_string());
            }
        } else {
            // Unquoted value — read up to whitespace or `>`.
            let end = rest
                .find(|c: char| c.is_whitespace() || c == '>' || c == '/')
                .unwrap_or(rest.len());
            return Some(rest[..end].to_string());
        }
        search = after_eq;
    }
    None
}

/// Compile an HTML file + CSS into a PRD document.
///
/// Flatten HTML for debug serving: resolve includes, page-content, extract inline CSS.
/// Returns (flattened_html, combined_css).
pub fn flatten_html_for_debug(
    html_source: &str,
    css_source: &str,
    base_dir: Option<&Path>,
) -> (String, String) {
    let html = preprocess_includes(html_source, base_dir, 0);
    let html = preprocess_page_content(&html, base_dir);
    let inline_css = extract_inline_styles(&html);
    let combined_css = if inline_css.is_empty() {
        css_source.to_string()
    } else if css_source.is_empty() {
        inline_css
    } else {
        format!("{}\n{}", css_source, inline_css)
    };
    (html, combined_css)
}

/// `html_source` — the HTML content.
/// `css_source` — the CSS content (from <link> or <style>).
/// `asset_dir` — base directory for resolving local asset paths.
/// `scene_type` — what kind of scene this is (wallpaper, widget, etc.).
pub fn compile_html(
    html_source: &str,
    css_source: &str,
    name: &str,
    scene_type: SceneType,
    asset_dir: Option<&Path>,
) -> anyhow::Result<(PrdDocument, Vec<ScriptBlock>, Vec<CssRule>)> {
    let mut doc = PrdDocument::new(name, scene_type);

    // 0. Preprocess <include> tags (with type/immediate support).
    let html_source = preprocess_includes(html_source, asset_dir, 0);
    // 0b. Preprocess <page-content> tags (inline default fragment).
    let html_source = preprocess_page_content(&html_source, asset_dir);
    // 0c. Extract redirect meta before tokenizer strips <meta> tags.
    doc.redirect = extract_redirect(&html_source);
    // 0d. Extract <title> tag (last one wins).
    doc.title = extract_title(&html_source);
    // 0e. Extract icon declarations from <meta name="icon"> tags emitted
    //     by the native <link rel="icon"> preprocessing.
    doc.icons = extract_icons(&html_source);
    let html_source = html_source.as_str();

    // 1. Extract inline <style> blocks from the HTML and merge with external CSS.
    //    This ensures CSS from both <head><style> and <body><style> blocks is captured,
    //    since the HTML tokenizer intentionally skips <style> tags.
    let inline_css = extract_inline_styles(html_source);
    let combined_css = if inline_css.is_empty() {
        css_source.to_string()
    } else if css_source.is_empty() {
        inline_css
    } else {
        format!("{}\n{}", css_source, inline_css)
    };

    // 1b. Parse CSS rules from the combined source.
    let rules = parse_css(&combined_css);

    // 2. Extract CSS custom properties (:root variables).
    let mut variables: HashMap<String, String> = HashMap::new();
    for rule in &rules {
        if rule.selector == ":root" {
            for (prop, val) in &rule.declarations {
                if prop.starts_with("--") {
                    variables.insert(prop.clone(), val.clone());
                }
            }
        }
    }
    doc.variables = variables.iter().map(|(k, v)| (k.clone(), v.clone())).collect();

    // Collect script blocks from the HTML.
    // Clear thread-local storage before tokenization.
    COLLECTED_SCRIPTS.with(|s| s.borrow_mut().clear());

    // 2b. Extract document background from body/html CSS rules.
    extract_document_background(&mut doc, &rules, &variables);

    // 3. Parse HTML into node tree.
    let tokens = tokenize_html(html_source);
    let (root_children, _) = build_node_tree(&tokens, 0, None);

    // 3b. Retrieve collected scripts.
    let scripts = COLLECTED_SCRIPTS.with(|s| s.borrow().clone());

    // 4. Add nodes to document and apply CSS.
    // Pass ancestor chain for descendant selector matching.
    let root_ancestors: Vec<AncestorInfo> = Vec::new();
    for child in root_children {
        let child_id = add_node_recursive(&mut doc, child, &rules, &variables, &root_ancestors, None);
        doc.add_child(doc.root, child_id);
    }

    // 5. Apply root styles.
    if let Some(root) = doc.get_node_mut(doc.root) {
        apply_rules_to_node(root, &rules, &variables);
    }

    // 6. CSS inheritance pass — propagate inheritable properties
    //    (color, font-family, font-size, font-weight, line-height,
    //     letter-spacing, text-align) from parent to child nodes.
    propagate_inherited_styles(&mut doc);

    // 7. Bundle assets from the asset directory and resolve <img src> references.
    if let Some(dir) = asset_dir {
        match crate::compiler::bundle::bundle_assets(&mut doc, dir) {
            Ok(path_to_index) => {
                crate::compiler::bundle::resolve_image_nodes(&mut doc, &path_to_index);
            }
            Err(e) => {
                log::warn!("[COMPILE] Asset bundling failed for {}: {}", dir.display(), e);
            }
        }

        // Load app-target icons into the asset bundle.
        let app_icons: Vec<(usize, String)> = doc.icons.iter().enumerate()
            .filter(|(_, icon)| icon.target == "app")
            .map(|(i, icon)| (i, icon.path.clone()))
            .collect();
        for (idx, icon_path) in app_icons {
            let p = std::path::Path::new(&icon_path);
            match crate::compiler::bundle::load_image_asset(&mut doc, p) {
                Ok(asset_idx) => {
                    doc.icons[idx].asset_index = Some(asset_idx);
                }
                Err(e) => {
                    log::warn!("[COMPILE] Failed to load app icon '{}': {}", icon_path, e);
                }
            }
        }
    }

    Ok((doc, scripts, rules))
}

/// Extract the content of all `<style>…</style>` blocks from an HTML string.
///
/// This captures CSS from both `<head><style>` and inline `<style>` blocks
/// within `<body>`, ensuring all CSS rules are available to the compiler.
/// The HTML tokenizer skips `<style>` tags, so this pre-extraction is required.
fn extract_inline_styles(html: &str) -> String {
    let mut css = String::new();
    let lower = html.to_lowercase();
    let mut search_start = 0;
    while let Some(open) = lower[search_start..].find("<style") {
        let abs_open = search_start + open;
        // Find the end of the opening tag '>'
        if let Some(gt) = lower[abs_open..].find('>') {
            let content_start = abs_open + gt + 1;
            if let Some(close) = lower[content_start..].find("</style>") {
                let content_end = content_start + close;
                css.push_str(&html[content_start..content_end]);
                css.push('\n');
                search_start = content_end + "</style>".len();
            } else {
                break;
            }
        } else {
            break;
        }
    }
    css
}

/// Extract document background color from body/html/:root CSS rules.
///
/// The od.default wallpaper uses `background: var(--bg-color)` on body,
/// which resolves to a hex color. We check body, html, and :root rules in
/// order, taking the last match (highest specificity).
fn extract_document_background(
    doc: &mut PrdDocument,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
) {
    use crate::compiler::css::resolve_var_pub;

    let bg_selectors = ["html", "body", ":root", "html,body", "html, body"];
    for rule in rules {
        let sel = rule.selector.trim();
        // Check if selector targets html, body, or :root.
        let matches = bg_selectors.iter().any(|s| sel == *s)
            || sel.split(',').any(|part| {
                let part = part.trim();
                part == "html" || part == "body"
            });

        if !matches {
            continue;
        }

        for (prop, val) in &rule.declarations {
            if prop == "background" || prop == "background-color" {
                let resolved = resolve_var_pub(val, variables);
                if let Some(color) = parse_color(&resolved) {
                    doc.background = color;
                }
            }
        }
    }
}

/// Propagate CSS-inherited properties from parent to child nodes.
///
/// In CSS, properties like `color`, `font-family`, `font-size`, `font-weight`,
/// `line-height`, `letter-spacing`, and `text-align` are inherited.  If a child
/// node doesn't have an explicitly-set value (still at default), it should
/// inherit from its parent.
///
/// We do a depth-first traversal, carrying the parent's style down.
fn propagate_inherited_styles(doc: &mut PrdDocument) {
    let defaults = ComputedStyle::default();
    let root_id = doc.root;

    // Collect root's inheritable props as initial values.
    let root_inherited = {
        let root = &doc.nodes[root_id as usize];
        InheritedProps::from_style(&root.style)
    };

    let children: Vec<u32> = doc.nodes[root_id as usize].children.clone();
    for child_id in children {
        propagate_recursive(doc, child_id, &root_inherited, &defaults);
    }
}

/// Inheritable CSS property bundle.
#[derive(Clone)]
struct InheritedProps {
    color: crate::prd::value::Color,
    font_family: String,
    font_size: f32,
    font_weight: FontWeight,
    line_height: f32,
    letter_spacing: f32,
    text_align: TextAlign,
    white_space: crate::prd::style::WhiteSpace,
    cursor: CursorStyle,
}

impl InheritedProps {
    fn from_style(s: &ComputedStyle) -> Self {
        Self {
            color: s.color,
            font_family: s.font_family.clone(),
            font_size: s.font_size,
            font_weight: s.font_weight,
            line_height: s.line_height,
            letter_spacing: s.letter_spacing,
            text_align: s.text_align,
            white_space: s.white_space,
            cursor: s.cursor,
        }
    }
}

fn propagate_recursive(
    doc: &mut PrdDocument,
    node_id: u32,
    parent: &InheritedProps,
    defaults: &ComputedStyle,
) {
    // Apply inherited values where the node still has the default.
    {
        let node = &mut doc.nodes[node_id as usize];
        // Color: inherit if still at the default (WHITE).
        if node.style.color == defaults.color {
            node.style.color = parent.color;
        }
        // font-family: inherit if empty (default).
        if node.style.font_family.is_empty() {
            node.style.font_family = parent.font_family.clone();
        }
        // font-size: inherit if same as default (16.0).
        if (node.style.font_size - defaults.font_size).abs() < 0.01 {
            node.style.font_size = parent.font_size;
        }
        // font-weight: inherit if default.
        if node.style.font_weight == defaults.font_weight {
            node.style.font_weight = parent.font_weight;
        }
        // line-height: inherit if default (1.5).
        if (node.style.line_height - defaults.line_height).abs() < 0.01 {
            node.style.line_height = parent.line_height;
        }
        // letter-spacing: inherit if zero (default).
        if node.style.letter_spacing.abs() < 0.001 {
            node.style.letter_spacing = parent.letter_spacing;
        }
        // text-align: inherit if default.
        if node.style.text_align == defaults.text_align {
            node.style.text_align = parent.text_align;
        }
        // white-space: inherit if default.
        if node.style.white_space == defaults.white_space {
            node.style.white_space = parent.white_space;
        }
        // cursor: inherit if still Auto (default).
        if node.style.cursor == defaults.cursor {
            node.style.cursor = parent.cursor;
        }
    }

    // Build inherited props from this node's current (post-inheritance) style.
    let my_inherited = {
        let node = &doc.nodes[node_id as usize];
        InheritedProps::from_style(&node.style)
    };

    let children: Vec<u32> = doc.nodes[node_id as usize].children.clone();
    for child_id in children {
        propagate_recursive(doc, child_id, &my_inherited, defaults);
    }
}

/// A temporary parsed HTML node before adding to the document.
struct ParsedNode {
    tag: String,
    classes: Vec<String>,
    id: Option<String>,
    attributes: HashMap<String, String>,
    inline_style: String,
    text_content: Option<String>,
    children: Vec<ParsedNode>,
}

/// Tokenize HTML into a flat list of events.
#[derive(Debug)]
enum HtmlToken {
    OpenTag {
        tag: String,
        attributes: HashMap<String, String>,
        self_closing: bool,
    },
    CloseTag {
        #[allow(dead_code)]
        tag: String,
    },
    Text(String),
}

fn tokenize_html(source: &str) -> Vec<HtmlToken> {
    let mut tokens = Vec::new();
    let mut pos = 0;
    let bytes = source.as_bytes();

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            // Check for comment
            if pos + 3 < bytes.len() && &source[pos..pos+4] == "<!--" {
                if let Some(end) = source[pos..].find("-->") {
                    pos += end + 3;
                    continue;
                }
            }

            // Check for <!DOCTYPE ...> — skip entirely, it's not a renderable element.
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'!' {
                // Skip to the closing >.
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                if pos < bytes.len() { pos += 1; } // skip >
                continue;
            }

            // Check for close tag
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
                pos += 2;
                let tag_start = pos;
                while pos < bytes.len() && bytes[pos] != b'>' {
                    pos += 1;
                }
                let tag = source[tag_start..pos].trim().to_lowercase();
                pos += 1; // skip >
                tokens.push(HtmlToken::CloseTag { tag });
                continue;
            }

            // Open tag
            pos += 1; // skip <
            let tag_start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                pos += 1;
            }
            let tag = source[tag_start..pos].trim().to_lowercase();

            // Parse attributes
            let mut attributes = HashMap::new();
            loop {
                // Skip whitespace
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() {
                    pos += 1;
                }
                if pos >= bytes.len() || bytes[pos] == b'>' || bytes[pos] == b'/' {
                    break;
                }

                // Attribute name
                let attr_start = pos;
                while pos < bytes.len() && bytes[pos] != b'=' && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                    pos += 1;
                }
                // Preserve original casing for attribute names.
                // SVG attributes like viewBox, preserveAspectRatio are case-sensitive
                // in XML, while HTML attributes are case-insensitive. We store as-is
                // and use case-insensitive lookups where needed.
                let attr_name = source[attr_start..pos].to_string();

                if pos < bytes.len() && bytes[pos] == b'=' {
                    pos += 1; // skip =
                    // Attribute value
                    let val = if pos < bytes.len() && (bytes[pos] == b'"' || bytes[pos] == b'\'') {
                        let quote = bytes[pos];
                        pos += 1;
                        let val_start = pos;
                        while pos < bytes.len() && bytes[pos] != quote {
                            pos += 1;
                        }
                        let val = source[val_start..pos].to_string();
                        if pos < bytes.len() { pos += 1; } // skip closing quote
                        val
                    } else {
                        let val_start = pos;
                        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' {
                            pos += 1;
                        }
                        source[val_start..pos].to_string()
                    };
                    attributes.insert(attr_name, val);
                } else {
                    attributes.insert(attr_name, String::new());
                }
            }

            let self_closing = pos < bytes.len() && bytes[pos] == b'/';
            if self_closing { pos += 1; }
            if pos < bytes.len() && bytes[pos] == b'>' { pos += 1; }

            // Skip <style>, <head>, <meta>, <link> tags entirely.
            // <script> tags are collected as ScriptBlock rather than skipped.
            let skip_tags = ["style", "head", "meta", "link", "title"];
            if skip_tags.contains(&tag.as_str()) {
                if !self_closing {
                    // Find closing tag
                    let close = format!("</{}>", tag);
                    if let Some(end) = source[pos..].to_lowercase().find(&close) {
                        pos += end + close.len();
                    }
                }
                continue;
            }

            // Collect <script> tags as ScriptBlock entries.
            if tag == "script" {
                let is_deferred = attributes.contains_key("data-deferred");
                if !self_closing {
                    let close = "</script>";
                    if let Some(end) = source[pos..].to_lowercase().find(close) {
                        let script_content = &source[pos..pos + end];
                        let src = attributes.get("src").cloned();
                        // Store script block in a thread-local for later retrieval.
                        COLLECTED_SCRIPTS.with(|s| {
                            s.borrow_mut().push(ScriptBlock {
                                content: script_content.trim().to_string(),
                                src,
                                deferred: is_deferred,
                            });
                        });
                        pos += end + close.len();
                    }
                } else if let Some(src) = attributes.get("src") {
                    COLLECTED_SCRIPTS.with(|s| {
                        s.borrow_mut().push(ScriptBlock {
                            content: String::new(),
                            src: Some(src.clone()),
                            deferred: is_deferred,
                        });
                    });
                }
                continue;
            }

            tokens.push(HtmlToken::OpenTag { tag, attributes, self_closing });
        } else {
            // Text content — collapse whitespace runs to a single space
            // but preserve boundary spaces for inline element spacing.
            let text_start = pos;
            while pos < bytes.len() && bytes[pos] != b'<' {
                pos += 1;
            }
            let raw = &source[text_start..pos];
            // Collapse internal whitespace runs to single spaces (CSS white-space: normal).
            let collapsed: String = raw.split_whitespace().collect::<Vec<_>>().join(" ");
            if !collapsed.is_empty() {
                // Preserve a leading space if the raw text started with whitespace
                // and a trailing space if the raw text ended with whitespace.
                // This is essential for inline element spacing (e.g. "text <strong>bold</strong>").
                let leading = raw.starts_with(char::is_whitespace) && !collapsed.is_empty();
                let trailing = raw.ends_with(char::is_whitespace) && !collapsed.is_empty();
                let mut text = String::with_capacity(collapsed.len() + 2);
                if leading { text.push(' '); }
                text.push_str(&collapsed);
                if trailing { text.push(' '); }
                tokens.push(HtmlToken::Text(text));
            }
        }
    }

    tokens
}

/// Build node tree from tokens.
///
/// The parser only unwinds recursion when it sees a close tag matching
/// `expected_close_tag`. Any unmatched/stray close tags are ignored so they
/// cannot prematurely truncate sibling layout trees.
fn build_node_tree(
    tokens: &[HtmlToken],
    start: usize,
    expected_close_tag: Option<&str>,
) -> (Vec<ParsedNode>, usize) {
    let mut nodes = Vec::new();
    let mut i = start;

    while i < tokens.len() {
        match &tokens[i] {
            HtmlToken::OpenTag { tag, attributes, self_closing } => {
                let classes: Vec<String> = attributes.get("class")
                    .map(|c| c.split_whitespace().map(String::from).collect())
                    .unwrap_or_default();
                let id = attributes.get("id").cloned();
                let inline_style = attributes.get("style").cloned().unwrap_or_default();

                let mut node = ParsedNode {
                    tag: tag.clone(),
                    classes,
                    id,
                    attributes: attributes.clone(),
                    inline_style,
                    text_content: None,
                    children: Vec::new(),
                };

                if *self_closing || is_void_element(tag) {
                    i += 1;
                } else {
                    let (children, end_pos) = build_node_tree(tokens, i + 1, Some(tag));
                    node.children = children;
                    i = end_pos + 1; // skip past the close tag
                }

                nodes.push(node);
            }
            HtmlToken::CloseTag { tag } => {
                // Only unwind when this close tag matches the currently-open tag.
                if expected_close_tag
                    .map(|expected| expected.eq_ignore_ascii_case(tag))
                    .unwrap_or(false)
                {
                    return (nodes, i);
                }

                // Ignore unmatched close tags at this depth; otherwise they can
                // incorrectly terminate parent parsing (e.g. explicit </path>).
                i += 1;
            }
            HtmlToken::Text(text) => {
                nodes.push(ParsedNode {
                    tag: "#text".to_string(),
                    classes: Vec::new(),
                    id: None,
                    attributes: HashMap::new(),
                    inline_style: String::new(),
                    text_content: Some(text.clone()),
                    children: Vec::new(),
                });
                i += 1;
            }
        }
    }

    (nodes, i)
}

fn is_void_element(tag: &str) -> bool {
    matches!(tag, "img" | "br" | "hr" | "input" | "meta" | "link" | "source"
        | "path" | "line" | "circle" | "rect" | "polyline" | "ellipse" | "polygon")
}

/// Info about an ancestor element, used for descendant selector matching.
#[derive(Clone)]
pub struct AncestorInfo {
    pub tag: Option<String>,
    pub classes: Vec<String>,
    pub html_id: Option<String>,
}

/// Add a parsed node tree to the PRD document.
fn add_node_recursive(
    doc: &mut PrdDocument,
    parsed: ParsedNode,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
    ancestors: &[AncestorInfo],
    parent_style: Option<&ComputedStyle>,
) -> NodeId {
    // ── SVG rasterization: convert <svg> elements to image nodes ──────
    if parsed.tag == "svg" {
        let current_color = parent_style.map(|ps| ps.color).unwrap_or(Color::WHITE);
        if let Some(asset_idx) = rasterize_svg_node(doc, &parsed, &current_color) {
            let kind = NodeKind::Image {
                asset_index: asset_idx,
                fit: ImageFit::Contain,
            };

            let mut style = ComputedStyle::default();
            style.display = Display::InlineBlock;

            // Inherit CSS properties from parent.
            if let Some(ps) = parent_style {
                style.color = ps.color;
                style.font_size = ps.font_size;
                style.font_family = ps.font_family.clone();
                style.font_weight = ps.font_weight;
                style.letter_spacing = ps.letter_spacing;
                style.line_height = ps.line_height;
                style.text_align = ps.text_align;
                style.text_transform = ps.text_transform;
            }

            // Use SVG width/height as intrinsic dimensions.
            if let Some(w) = parsed.attributes.get("width").and_then(|v| parse_svg_length(v)) {
                style.width = Dimension::Px(w);
            }
            if let Some(h) = parsed.attributes.get("height").and_then(|v| parse_svg_length(v)) {
                style.height = Dimension::Px(h);
            }

            // Fallback to viewBox dimensions when width/height are not specified.
            if matches!(style.width, Dimension::Auto) || matches!(style.height, Dimension::Auto) {
                let vb = parsed.attributes.get("viewBox")
                    .or_else(|| parsed.attributes.get("viewbox"));
                if let Some(vb_str) = vb {
                    if let Some((vw, vh)) = parse_viewbox_dims(vb_str) {
                        if matches!(style.width, Dimension::Auto) {
                            style.width = Dimension::Px(vw);
                        }
                        if matches!(style.height, Dimension::Auto) {
                            style.height = Dimension::Px(vh);
                        }
                    }
                }
            }

            // Wire the rasterized image into style.background so the
            // paint system actually renders the texture.
            style.background = Background::Image { asset_index: asset_idx };

            let mut node = PrdNode {
                id: 0,
                tag: Some("svg".to_string()),
                html_id: parsed.id.clone(),
                classes: parsed.classes.clone(),
                attributes: parsed.attributes.clone(),
                kind,
                style,
                children: Vec::new(),
                events: extract_event_bindings(&parsed),
                animations: Vec::new(),
                layout: Default::default(),
                hover_style: Vec::new(),
                active_style: Vec::new(),
                focus_style: Vec::new(),
                hovered: false,
                active: false,
                focused: false,
            };

            // Apply CSS rules.
            let html_id = parsed.id.clone();
            apply_rules_to_node_with_ancestors(&mut node, &html_id, rules, variables, ancestors);

            // Apply inline styles.
            if !parsed.inline_style.is_empty() {
                for decl in parsed.inline_style.split(';') {
                    let decl = decl.trim();
                    if let Some((prop, val)) = decl.split_once(':') {
                        apply_property(&mut node.style, prop.trim(), val.trim(), variables);
                    }
                }
            }

            // Restore the rasterized image background if CSS overwrote it
            // with a solid color (e.g. `background: transparent`).
            if !matches!(node.style.background, Background::Image { .. }) {
                node.style.background = Background::Image { asset_index: asset_idx };
            }

            return doc.add_node(node);
        }
        // Fall through to normal handling if rasterization fails.
    }

    let kind = determine_node_kind(&parsed, variables);

    // For widget elements, children are consumed by the widget (label, options, etc.)
    // and should not be added as child scene nodes.
    let skip_children = matches!(&kind,
        NodeKind::Input(InputKind::Button { .. }) |
        NodeKind::Input(InputKind::Dropdown { .. }) |
        NodeKind::Input(InputKind::TextArea { .. })
    );

    let mut style = ComputedStyle::default();

    // Tag-specific display defaults — mirrors HTML's block/inline model.
    // Inline-level tags default to flex-row so their children flow horizontally,
    // which is the closest equivalent to inline-flow in our block/flex engine.
    match parsed.tag.as_str() {
        "#text" | "span" | "a" | "label" | "code" | "small" => {
            style.display = Display::InlineBlock;
        }
        // Inline + bold
        "strong" | "b" => {
            style.display = Display::InlineBlock;
            style.font_weight = FontWeight(700);
        }
        // Inline + italic (note: we store italic as weight 0 marker; see text painter)
        "em" | "i" => {
            style.display = Display::InlineBlock;
            // Italic handled via tag check in text painter; no weight change.
        }
        // Heading defaults — browser UA sizes relative to 16px base.
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.67);
            style.margin.bottom = Dimension::Em(0.67);
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.83);
            style.margin.bottom = Dimension::Em(0.83);
        }
        "h3" => {
            style.font_size = 18.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        "h4" => {
            style.font_size = 16.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.33);
            style.margin.bottom = Dimension::Em(1.33);
        }
        "h5" => {
            style.font_size = 13.28;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.67);
            style.margin.bottom = Dimension::Em(1.67);
        }
        "h6" => {
            style.font_size = 10.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(2.33);
            style.margin.bottom = Dimension::Em(2.33);
        }
        // <p> uses flex-row-wrap so inline children (text, <strong>, <em>, etc.)
        // flow horizontally, approximating CSS inline formatting context.
        "p" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
            style.flex_wrap = crate::prd::style::FlexWrap::Wrap;
            style.align_items = AlignItems::FlexStart;
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        // data-bind custom tag default.
        "data-bind" => {
            style.display = Display::InlineBlock;
            if parsed.classes.iter().any(|c| c == "val") {
                style.flex_grow = 1.0;
            }
        }
        // data-bar custom tag default.
        "data-bar" => {
            style.display = Display::Block;
        }
        // canvas element — block-level, sized by width/height attributes.
        "canvas" => {
            style.display = Display::Block;
            // Use HTML width/height attributes as intrinsic CSS dimensions,
            // matching browser behavior where canvas elements have
            // intrinsic size from their attributes (default 300×150).
            if let Some(w) = parsed.attributes.get("width").and_then(|v| v.parse::<f32>().ok()) {
                style.width = Dimension::Px(w);
            }
            if let Some(h) = parsed.attributes.get("height").and_then(|v| v.parse::<f32>().ok()) {
                style.height = Dimension::Px(h);
            }
        }
        // Semantic block-level elements (HTML5) — treated as block containers
        // by browser UA stylesheets, need explicit listing here.
        "nav" | "section" | "header" | "footer" | "article" | "main" | "aside" | "figure" | "figcaption" | "ul" | "ol" | "li" | "page-content" => {
            style.display = Display::Block;
        }
        _ => {} // default Block
    }

    // Inherit CSS properties from parent (CSS inheritance model).
    if let Some(ps) = parent_style {
        style.color = ps.color;
        style.font_size = ps.font_size;
        style.font_family = ps.font_family.clone();
        style.font_weight = ps.font_weight;
        style.letter_spacing = ps.letter_spacing;
        style.line_height = ps.line_height;
        style.text_align = ps.text_align;
        style.text_transform = ps.text_transform;
    }

    let mut node = PrdNode {
        id: 0, // Will be set by add_node
        tag: Some(parsed.tag.clone()),
        html_id: parsed.id.clone(),
        classes: parsed.classes.clone(),
        attributes: parsed.attributes.clone(),
        kind,
        style,
        children: Vec::new(),
        events: extract_event_bindings(&parsed),
        animations: Vec::new(),
        layout: Default::default(),
        hover_style: Vec::new(),
        active_style: Vec::new(),
        focus_style: Vec::new(),
        hovered: false,
        active: false,
        focused: false,
    };

    // Keep the raw id for selector matching.
    let html_id = parsed.id.clone();

    // Apply CSS rules in order with ancestor-aware matching.
    apply_rules_to_node_with_ancestors(&mut node, &html_id, rules, variables, ancestors);

    // Apply inline styles (highest specificity).
    if !parsed.inline_style.is_empty() {
        for decl in parsed.inline_style.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                apply_property(&mut node.style, prop.trim(), val.trim(), variables);
            }
        }
    }

    // Snapshot the finalized style for children to inherit from.
    let inherited_style = node.style.clone();
    let node_id = doc.add_node(node);

    // Build ancestor info for children.
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        tag: Some(parsed.tag.clone()),
        classes: parsed.classes.clone(),
        html_id,
    });

    // Add children (unless consumed by widget).
    if !skip_children {
        for child in parsed.children {
            let child_id = add_node_recursive(doc, child, rules, variables, &child_ancestors, Some(&inherited_style));
            doc.add_child(node_id, child_id);
        }
    }

    node_id
}

/// Extract text content from direct #text children of a parsed node.
fn extract_text_content(parsed: &ParsedNode) -> String {
    let mut text = String::new();
    for child in &parsed.children {
        if child.tag == "#text" {
            if let Some(t) = &child.text_content {
                if !text.is_empty() { text.push(' '); }
                text.push_str(t);
            }
        }
    }
    text
}

/// Extract <option> children from a <select> element.
fn extract_select_options(parsed: &ParsedNode) -> Vec<(String, String)> {
    let mut options = Vec::new();
    for child in &parsed.children {
        if child.tag == "option" {
            let value = child.attributes.get("value").cloned().unwrap_or_default();
            let label = extract_text_content(child);
            let label = if label.is_empty() { value.clone() } else { label };
            options.push((value, label));
        }
    }
    options
}

/// Extract event bindings from data-* attributes.
fn extract_event_bindings(parsed: &ParsedNode) -> Vec<EventBinding> {
    let mut events = Vec::new();

    // data-action with optional data-event (defaults to "click")
    if let Some(action_type) = parsed.attributes.get("data-action") {
        let event_type = parsed.attributes.get("data-event")
            .cloned()
            .unwrap_or_else(|| "click".to_string());

        let action = match action_type.as_str() {
            "navigate" => {
                let target = parsed.attributes.get("data-target")
                    .cloned().unwrap_or_default();
                EventAction::Navigate { scene_id: target }
            }
            "ipc" => {
                let ns = parsed.attributes.get("data-ns")
                    .cloned().unwrap_or_default();
                let cmd = parsed.attributes.get("data-cmd")
                    .cloned().unwrap_or_default();
                let args = parsed.attributes.get("data-args")
                    .and_then(|a| serde_json::from_str(a).ok());
                EventAction::IpcCommand { ns, cmd, args }
            }
            "toggle-class" => {
                let class = parsed.attributes.get("data-class")
                    .cloned().unwrap_or_default();
                // target: resolved later; 0 means "self" (the node with this binding).
                // data-target can specify an HTML id to resolve at runtime.
                let target_id = parsed.attributes.get("data-target")
                    .cloned().unwrap_or_default();
                EventAction::ToggleClass { target: 0, class, target_html_id: target_id }
            }
            "window-close" => EventAction::WindowClose,
            "window-minimize" => EventAction::WindowMinimize,
            "window-maximize" => EventAction::WindowMaximize,
            "window-drag" => EventAction::WindowDrag,
            _ => {
                // Treat the action string as an IPC command name.
                EventAction::IpcCommand {
                    ns: String::new(),
                    cmd: action_type.clone(),
                    args: None,
                }
            }
        };

        events.push(EventBinding { event: event_type, action });
    }

    // data-navigate shorthand
    if let Some(target) = parsed.attributes.get("data-navigate") {
        events.push(EventBinding {
            event: "click".to_string(),
            action: EventAction::Navigate { scene_id: target.clone() },
        });
    }

    events
}

// ── SVG rasterization ──────────────────────────────────────────────────────

/// Reconstruct SVG markup from a parsed `<svg>` node and its children.
fn reconstruct_svg_markup(node: &ParsedNode) -> String {
    let mut svg = String::with_capacity(512);
    reconstruct_svg_element(&mut svg, node);
    svg
}

fn reconstruct_svg_element(out: &mut String, node: &ParsedNode) {
    if node.tag == "#text" {
        if let Some(ref text) = node.text_content {
            out.push_str(text);
        }
        return;
    }

    out.push('<');
    out.push_str(&node.tag);

    // For the root <svg>, ensure the xmlns attribute is present.
    if node.tag == "svg" && !node.attributes.contains_key("xmlns") {
        out.push_str(" xmlns=\"http://www.w3.org/2000/svg\"");
    }

    for (key, val) in &node.attributes {
        // Skip class/id/style — they're CSS concerns, not SVG rendering.
        if key == "class" || key == "id" || key == "style" { continue; }
        out.push(' ');
        out.push_str(key);
        out.push_str("=\"");
        out.push_str(val);
        out.push('"');
    }

    if node.children.is_empty() && is_void_element(&node.tag) {
        out.push_str(" />");
    } else {
        out.push('>');
        for child in &node.children {
            reconstruct_svg_element(out, child);
        }
        out.push_str("</");
        out.push_str(&node.tag);
        out.push('>');
    }
}

/// Rasterize an SVG string to RGBA pixels using resvg.
/// Returns (rgba_bytes, width, height) on success.
pub fn rasterize_svg(svg_markup: &str, target_w: u32, target_h: u32) -> Option<(Vec<u8>, u32, u32)> {
    let options = resvg::usvg::Options::default();
    let tree = match resvg::usvg::Tree::from_str(svg_markup, &options) {
        Ok(t) => t,
        Err(e) => {
            log::warn!("[SVG] Failed to parse SVG: {}", e);
            return None;
        }
    };

    let tree_size = tree.size();
    let svg_w = tree_size.width();
    let svg_h = tree_size.height();
    if svg_w <= 0.0 || svg_h <= 0.0 {
        return None;
    }

    // Render at target size (or SVG intrinsic size if no target specified).
    let render_w = if target_w > 0 { target_w } else { svg_w.ceil() as u32 };
    let render_h = if target_h > 0 { target_h } else { svg_h.ceil() as u32 };
    let render_w = render_w.max(1);
    let render_h = render_h.max(1);

    let scale_x = render_w as f32 / svg_w;
    let scale_y = render_h as f32 / svg_h;

    let mut pixmap = resvg::tiny_skia::Pixmap::new(render_w, render_h)?;
    let transform = resvg::tiny_skia::Transform::from_scale(scale_x, scale_y);
    resvg::render(&tree, transform, &mut pixmap.as_mut());

    // resvg outputs premultiplied RGBA — keep it as-is since the GPU
    // pipeline uses PREMULTIPLIED_ALPHA_BLENDING. Converting to straight
    // alpha here would cause over-bright fringes on semi-transparent edges.
    let rgba = pixmap.take();

    Some((rgba, render_w, render_h))
}

/// Rasterize an `<svg>` ParsedNode, add result as an image asset to the doc.
/// Returns the asset index on success.
fn rasterize_svg_node(doc: &mut PrdDocument, parsed: &ParsedNode, current_color: &Color) -> Option<u32> {
    let color_hex = current_color.to_hex_string();
    let mut svg_markup = reconstruct_svg_markup(parsed);
    svg_markup = svg_markup.replace("currentColor", &color_hex);
    svg_markup = svg_markup.replace("currentcolor", &color_hex);

    // Parse target dimensions from width/height attributes (CSS may override later).
    let mut target_w: u32 = parsed
        .attributes
        .get("width")
        .and_then(|v| parse_svg_length(v))
        .map(|v| v.max(1.0) as u32)
        .unwrap_or(0);
    let mut target_h: u32 = parsed
        .attributes
        .get("height")
        .and_then(|v| parse_svg_length(v))
        .map(|v| v.max(1.0) as u32)
        .unwrap_or(0);

    // Fall back to viewBox dimensions when width/height aren't provided.
    if (target_w == 0 || target_h == 0) && parsed.attributes.get("viewBox").is_some() {
        if let Some((vb_w, vb_h)) = parsed
            .attributes
            .get("viewBox")
            .and_then(|vb| parse_viewbox_dims(vb))
        {
            if target_w == 0 { target_w = vb_w.max(1.0) as u32; }
            if target_h == 0 { target_h = vb_h.max(1.0) as u32; }
        }
    }

    if (target_w == 0 || target_h == 0) && parsed.attributes.get("viewbox").is_some() {
        if let Some((vb_w, vb_h)) = parsed
            .attributes
            .get("viewbox")
            .and_then(|vb| parse_viewbox_dims(vb))
        {
            if target_w == 0 { target_w = vb_w.max(1.0) as u32; }
            if target_h == 0 { target_h = vb_h.max(1.0) as u32; }
        }
    }

    // Last-resort fallback keeps icon-only SVGs visible even when attributes are omitted.
    if target_w == 0 { target_w = 24; }
    if target_h == 0 { target_h = 24; }

    // Render at 2x for crisp display on high-DPI screens.
    let scale = 2u32;
    let raster_w = target_w.saturating_mul(scale).max(1);
    let raster_h = target_h.saturating_mul(scale).max(1);

    let (rgba, w, h) = match rasterize_svg(&svg_markup, raster_w, raster_h) {
        Some(ok) => ok,
        None => {
            log::warn!(
                "[SVG] Rasterization failed (target={}x{}, attrs={:?})",
                target_w,
                target_h,
                parsed.attributes
            );
            return None;
        }
    };

    let name = format!("svg_raster_{}x{}", w, h);
    let idx = doc.assets.add_raw_image(name, w, h, rgba);
    log::info!(
        "[SVG] Rasterized '{}' to {}x{} (asset #{})",
        parsed.tag,
        w,
        h,
        idx
    );
    Some(idx)
}

fn parse_svg_length(value: &str) -> Option<f32> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Accept plain numeric values and values with units (e.g. "24px").
    let numeric: String = trimmed
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == '-')
        .collect();

    if numeric.is_empty() {
        return None;
    }

    numeric.parse::<f32>().ok()
}

fn parse_viewbox_dims(vb: &str) -> Option<(f32, f32)> {
    let nums: Vec<f32> = vb
        .split(|c: char| c.is_ascii_whitespace() || c == ',')
        .filter(|s| !s.is_empty())
        .filter_map(|s| s.parse::<f32>().ok())
        .collect();

    if nums.len() == 4 {
        Some((nums[2], nums[3]))
    } else {
        None
    }
}

/// Determine the NodeKind from the HTML element.
fn determine_node_kind(parsed: &ParsedNode, _variables: &HashMap<String, String>) -> NodeKind {
    match parsed.tag.as_str() {
        "#text" => {
            NodeKind::Text {
                content: parsed.text_content.clone().unwrap_or_default(),
            }
        }
        "img" => {
            NodeKind::Image {
                asset_index: 0, // Will be resolved during asset bundling.
                fit: ImageFit::Cover,
            }
        }
        "data-bind" | "data-bar" => NodeKind::Container,
        "page-content" => NodeKind::PageContent,
        "canvas" => {
            let width: u32 = parsed.attributes.get("width")
                .and_then(|v| v.parse().ok())
                .unwrap_or(300);
            let height: u32 = parsed.attributes.get("height")
                .and_then(|v| v.parse().ok())
                .unwrap_or(150);
            NodeKind::Canvas { width, height }
        }
        "path" => {
            let d = parsed.attributes.get("d").cloned().unwrap_or_default();
            NodeKind::SvgPath {
                d,
                stroke_color: None,
                fill_color: None,
                stroke_width: 2.0,
            }
        }

        // ── Interactive elements ────────────────────────────────────

        "button" => {
            // If the button has non-text children (e.g. SVG icons), treat it
            // as a plain container so the children are rendered normally.
            let has_complex_children = parsed.children.iter().any(|c| c.tag != "#text");
            if has_complex_children {
                NodeKind::Container
            } else {
                let label = extract_text_content(parsed);
                let disabled = parsed.attributes.contains_key("disabled");
                let variant = match parsed.attributes.get("data-variant").map(|s| s.as_str()) {
                    Some("primary") => ButtonVariant::Primary,
                    Some("secondary") => ButtonVariant::Secondary,
                    Some("danger") => ButtonVariant::Danger,
                    Some("ghost") => ButtonVariant::Ghost,
                    Some("link") => ButtonVariant::Link,
                    _ => ButtonVariant::Primary,
                };
                NodeKind::Input(InputKind::Button { label, disabled, variant })
            }
        }

        "input" => {
            let input_type = parsed.attributes.get("type")
                .map(|s| s.as_str()).unwrap_or("text");
            match input_type {
                "checkbox" => {
                    let checked = parsed.attributes.contains_key("checked");
                    let disabled = parsed.attributes.contains_key("disabled");
                    let label = parsed.attributes.get("data-label")
                        .cloned().unwrap_or_default();
                    let style = match parsed.attributes.get("data-style").map(|s| s.as_str()) {
                        Some("toggle") => CheckboxStyle::Toggle,
                        _ => CheckboxStyle::Checkbox,
                    };
                    NodeKind::Input(InputKind::Checkbox { label, checked, disabled, style })
                }
                "range" => {
                    let value = parsed.attributes.get("value")
                        .and_then(|v| v.parse().ok()).unwrap_or(50.0);
                    let min = parsed.attributes.get("min")
                        .and_then(|v| v.parse().ok()).unwrap_or(0.0);
                    let max = parsed.attributes.get("max")
                        .and_then(|v| v.parse().ok()).unwrap_or(100.0);
                    let step = parsed.attributes.get("step")
                        .and_then(|v| v.parse().ok()).unwrap_or(1.0);
                    let disabled = parsed.attributes.contains_key("disabled");
                    NodeKind::Input(InputKind::Slider {
                        value, min, max, step, disabled, show_value: true,
                    })
                }
                _ => {
                    // text, password, number, email, search
                    let placeholder = parsed.attributes.get("placeholder")
                        .cloned().unwrap_or_default();
                    let value = parsed.attributes.get("value")
                        .cloned().unwrap_or_default();
                    let max_length = parsed.attributes.get("maxlength")
                        .and_then(|v| v.parse().ok()).unwrap_or(0);
                    let read_only = parsed.attributes.contains_key("readonly");
                    let kind = match input_type {
                        "password" => TextInputType::Password,
                        "number"   => TextInputType::Number,
                        "email"    => TextInputType::Email,
                        "search"   => TextInputType::Search,
                        _          => TextInputType::Text,
                    };
                    NodeKind::Input(InputKind::TextInput {
                        placeholder, value, max_length, read_only, input_type: kind,
                    })
                }
            }
        }

        "select" => {
            let options = extract_select_options(parsed);
            let selected = parsed.attributes.get("value").cloned();
            let placeholder = parsed.attributes.get("placeholder")
                .cloned().unwrap_or_else(|| "Select...".to_string());
            let disabled = parsed.attributes.contains_key("disabled");
            NodeKind::Input(InputKind::Dropdown {
                options, selected, placeholder, disabled, open: false,
            })
        }

        "textarea" => {
            let placeholder = parsed.attributes.get("placeholder")
                .cloned().unwrap_or_default();
            let value = extract_text_content(parsed);
            let max_length = parsed.attributes.get("maxlength")
                .and_then(|v| v.parse().ok()).unwrap_or(0);
            let read_only = parsed.attributes.contains_key("readonly");
            let rows = parsed.attributes.get("rows")
                .and_then(|v| v.parse().ok()).unwrap_or(4);
            NodeKind::Input(InputKind::TextArea {
                placeholder, value, max_length, read_only, rows,
            })
        }

        _ => NodeKind::Container,
    }
}

/// Re-apply CSS rules to all nodes in a document (for runtime class toggling).
/// Resets each node's style to tag defaults, applies CSS rules, then inline styles.
/// Finishes with inherited style propagation.
pub fn reapply_all_styles(doc: &mut PrdDocument, rules: &[CssRule]) {
    let variables: HashMap<String, String> = doc.variables.iter().cloned().collect();
    let root = doc.root;
    // Collect the children list from root (we need to borrow doc mutably later).
    let root_children: Vec<NodeId> = doc.get_node(root).map(|n| n.children.clone()).unwrap_or_default();

    // Reset and re-apply root node styles.
    if let Some(node) = doc.get_node_mut(root) {
        let saved_bg = if matches!(node.kind, NodeKind::Image { .. }) {
            Some(node.style.background.clone())
        } else {
            None
        };
        let tag = node.tag.as_deref().unwrap_or("").to_string();
        node.style = tag_default_style(&tag);
        node.hover_style.clear();
        node.active_style.clear();
        node.focus_style.clear();
        let html_id = node.html_id.clone();
        apply_rules_to_node_with_ancestors(node, &html_id, rules, &variables, &[]);
        reapply_inline_styles(node, &variables);
        if let Some(bg) = saved_bg {
            if !matches!(node.style.background, Background::Image { .. }) {
                node.style.background = bg;
            }
        }
    }

    let root_tag = doc.get_node(root).map(|n| n.tag.clone()).unwrap_or(None);
    let root_classes = doc.get_node(root).map(|n| n.classes.clone()).unwrap_or_default();
    let root_html_id = doc.get_node(root).and_then(|n| n.html_id.clone());
    let root_ancestor = AncestorInfo { tag: root_tag, classes: root_classes, html_id: root_html_id };

    for child_id in root_children {
        reapply_styles_recursive(doc, child_id, rules, &variables, &[root_ancestor.clone()]);
    }

    // Re-propagate inherited styles (color, font, etc.) from parent to child.
    propagate_inherited_styles(doc);
}

fn reapply_styles_recursive(
    doc: &mut PrdDocument,
    node_id: NodeId,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
    ancestors: &[AncestorInfo],
) {
    // Collect info we need before borrowing doc mutably.
    let (children, tag, classes, html_id) = {
        let node = match doc.get_node(node_id) {
            Some(n) => n,
            None => return,
        };
        (node.children.clone(), node.tag.clone(), node.classes.clone(), node.html_id.clone())
    };

    // Reset to tag defaults, apply CSS rules, then inline styles.
    if let Some(node) = doc.get_node_mut(node_id) {
        // Preserve the rasterized image backing before reset — rasterized SVG
        // nodes have kind=NodeKind::Image but no CSS property can restore that
        // background, so we must keep it through the style reset.
        let saved_bg = if matches!(node.kind, NodeKind::Image { .. }) {
            Some(node.style.background.clone())
        } else {
            None
        };
        let tag_str = node.tag.as_deref().unwrap_or("");
        node.style = tag_default_style(tag_str);
        node.hover_style.clear();
        node.active_style.clear();
        node.focus_style.clear();
        apply_rules_to_node_with_ancestors(node, &html_id, rules, variables, ancestors);
        reapply_inline_styles(node, variables);
        // Restore the image background if CSS didn't supply one.
        if let Some(bg) = saved_bg {
            if !matches!(node.style.background, Background::Image { .. }) {
                node.style.background = bg;
            }
        }
    }

    // Build ancestor chain for children.
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo { tag, classes, html_id });

    for child_id in children {
        reapply_styles_recursive(doc, child_id, rules, variables, &child_ancestors);
    }
}

/// Re-apply inline `style=""` attributes (highest CSS specificity).
fn reapply_inline_styles(node: &mut PrdNode, variables: &HashMap<String, String>) {
    if let Some(inline) = node.attributes.get("style").cloned() {
        for decl in inline.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                apply_property(&mut node.style, prop.trim(), val.trim(), variables);
            }
        }
    }
}

/// Get the default ComputedStyle for an HTML tag (mirrors add_node_recursive logic).
fn tag_default_style(tag: &str) -> ComputedStyle {
    let mut style = ComputedStyle::default();
    match tag {
        "#text" | "span" | "a" | "label" | "code" | "small" => {
            style.display = Display::InlineBlock;
        }
        "strong" | "b" => {
            style.display = Display::InlineBlock;
            style.font_weight = FontWeight(700);
        }
        "em" | "i" => {
            style.display = Display::InlineBlock;
        }
        "h1" => {
            style.font_size = 32.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.67);
            style.margin.bottom = Dimension::Em(0.67);
        }
        "h2" => {
            style.font_size = 24.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(0.83);
            style.margin.bottom = Dimension::Em(0.83);
        }
        "h3" => {
            style.font_size = 18.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        "h4" => {
            style.font_size = 16.0;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.33);
            style.margin.bottom = Dimension::Em(1.33);
        }
        "h5" => {
            style.font_size = 13.28;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(1.67);
            style.margin.bottom = Dimension::Em(1.67);
        }
        "h6" => {
            style.font_size = 10.72;
            style.font_weight = FontWeight(700);
            style.margin.top = Dimension::Em(2.33);
            style.margin.bottom = Dimension::Em(2.33);
        }
        "p" => {
            style.display = Display::Flex;
            style.flex_direction = FlexDirection::Row;
            style.flex_wrap = crate::prd::style::FlexWrap::Wrap;
            style.align_items = AlignItems::FlexStart;
            style.margin.top = Dimension::Em(1.0);
            style.margin.bottom = Dimension::Em(1.0);
        }
        "data-bind" => {
            style.display = Display::InlineBlock;
        }
        "data-bar" => {
            style.display = Display::Block;
        }
        "nav" | "section" | "header" | "footer" | "article" | "main" | "aside" | "figure" | "figcaption" | "ul" | "ol" | "li" | "page-content" => {
            style.display = Display::Block;
        }
        _ => {}
    }
    style
}

/// Apply matching CSS rules to a node with ancestor context for descendant matching.
pub fn apply_rules_to_node_with_ancestors(
    node: &mut PrdNode,
    html_id: &Option<String>,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
    ancestors: &[AncestorInfo],
) {
    for rule in rules {
        if compound_selector_matches(&rule.compound_selectors, node, html_id, ancestors) {
            if let Some(ref pseudo) = rule.pseudo_class {
                if pseudo == "hover" {
                    for (prop, val) in &rule.declarations {
                        let resolved = crate::compiler::css::resolve_var_pub(val, variables);
                        node.hover_style.push((prop.clone(), resolved));
                    }
                } else if pseudo == "active" {
                    for (prop, val) in &rule.declarations {
                        let resolved = crate::compiler::css::resolve_var_pub(val, variables);
                        node.active_style.push((prop.clone(), resolved));
                    }
                } else if pseudo == "focus" || pseudo == "focus-visible" || pseudo == "focus-within" {
                    for (prop, val) in &rule.declarations {
                        let resolved = crate::compiler::css::resolve_var_pub(val, variables);
                        node.focus_style.push((prop.clone(), resolved));
                    }
                }
            } else {
                for (prop, val) in &rule.declarations {
                    apply_property(&mut node.style, prop, val, variables);
                }
            }
        }
    }
}

/// Apply matching CSS rules to a node (legacy, for root node without ancestors).
fn apply_rules_to_node(
    node: &mut PrdNode,
    rules: &[CssRule],
    variables: &HashMap<String, String>,
) {
    let no_ancestors: Vec<AncestorInfo> = Vec::new();
    apply_rules_to_node_with_ancestors(node, &None, rules, variables, &no_ancestors);
}

/// Check if a compound selector chain matches a node within its ancestor context.
///
/// The last compound selector must match the node itself.
/// Earlier compound selectors must match ancestors (in order, not necessarily consecutive).
pub fn compound_selector_matches(
    selectors: &[CompoundSelector],
    node: &PrdNode,
    html_id: &Option<String>,
    ancestors: &[AncestorInfo],
) -> bool {
    if selectors.is_empty() {
        return false;
    }

    // The last compound must match the node itself.
    let last = &selectors[selectors.len() - 1];
    if !last.matches_node(
        node.tag.as_deref(),
        &node.classes,
        html_id.as_deref(),
    ) {
        return false;
    }

    // If there's only one compound, we're done (simple selector).
    if selectors.len() == 1 {
        return true;
    }

    // Walk remaining selectors right-to-left, matching against ancestors.
    // Descendant combinator: ancestor doesn't need to be direct parent.
    let remaining = &selectors[..selectors.len() - 1];
    let mut sel_idx = remaining.len() as i32 - 1;
    let mut anc_idx = ancestors.len() as i32 - 1;

    while sel_idx >= 0 && anc_idx >= 0 {
        let sel = &remaining[sel_idx as usize];
        let anc = &ancestors[anc_idx as usize];
        if sel.matches_node(anc.tag.as_deref(), &anc.classes, anc.html_id.as_deref()) {
            sel_idx -= 1;
        }
        anc_idx -= 1;
    }

    // All ancestor selectors must have been matched.
    sel_idx < 0
}

// ═══════════════════════════════════════════════════════════════════════════
// Post-JS CSS restyle pass
// ═══════════════════════════════════════════════════════════════════════════

/// Re-apply CSS rules to every node in the document using the full
/// compile-time pipeline (tag defaults → parent inherit → compound-selector
/// matching → inline styles).  Call this after JS has finished mutating the
/// DOM so that dynamically-created nodes receive proper styling.
pub fn restyle_document(
    doc: &mut PrdDocument,
    rules: &[CssRule],
    variables: &std::collections::HashMap<String, String>,
) {
    let root = doc.root;
    restyle_recursive(doc, root, rules, variables, &[], None);
}

fn restyle_recursive(
    doc: &mut PrdDocument,
    node_id: NodeId,
    rules: &[CssRule],
    variables: &std::collections::HashMap<String, String>,
    ancestors: &[AncestorInfo],
    parent_style: Option<&ComputedStyle>,
) {
    // ── 1. Read node metadata (immutable borrow) ──────────────────────
    let (tag, classes, html_id, inline_style, children, canvas_dims) = {
        let node = match doc.get_node(node_id) {
            Some(n) => n,
            None => return,
        };
        let cdims = if let crate::prd::node::NodeKind::Canvas { width, height } = &node.kind {
            Some((*width, *height))
        } else {
            None
        };
        (
            node.tag.clone(),
            node.classes.clone(),
            node.html_id.clone(),
            // Reconstruct inline style from the node's `style` attribute if present.
            node.attributes.get("style").cloned().unwrap_or_default(),
            node.children.clone(),
            cdims,
        )
    };

    // ── 2. Build fresh style: defaults → tag defaults → inherit → CSS rules → inline ─
    let mut style = ComputedStyle::default();

    // Tag-specific display defaults (mirrors add_node_recursive).
    if let Some(ref t) = tag {
        match t.as_str() {
            "#text" | "span" | "a" | "label" | "code" | "small" => {
                style.display = Display::InlineBlock;
            }
            "strong" | "b" => {
                style.display = Display::InlineBlock;
                style.font_weight = FontWeight(700);
            }
            "em" | "i" => {
                style.display = Display::InlineBlock;
            }
            "h1" => { style.font_size = 32.0; style.font_weight = FontWeight(700); }
            "h2" => { style.font_size = 24.0; style.font_weight = FontWeight(700); }
            "h3" => { style.font_size = 18.72; style.font_weight = FontWeight(700); }
            "h4" => { style.font_size = 16.0; style.font_weight = FontWeight(700); }
            "h5" => { style.font_size = 13.28; style.font_weight = FontWeight(700); }
            "h6" => { style.font_size = 10.72; style.font_weight = FontWeight(700); }
            "p" => {
                style.display = Display::Flex;
                style.flex_direction = FlexDirection::Row;
                style.flex_wrap = crate::prd::style::FlexWrap::Wrap;
                style.align_items = AlignItems::FlexStart;
            }
            "data-bind" => {
                style.display = crate::prd::style::Display::InlineBlock;
                if classes.iter().any(|c| c == "val") {
                    style.flex_grow = 1.0;
                }
            }
            "data-bar" => {
                style.display = crate::prd::style::Display::Block;
            }
            "canvas" => {
                style.display = Display::Block;
                // Use canvas buffer dimensions as intrinsic CSS size,
                // matching browser behavior.
                if let Some((w, h)) = canvas_dims {
                    style.width = Dimension::Px(w as f32);
                    style.height = Dimension::Px(h as f32);
                }
            }
            _ => {} // default Block
        }
    }

    // Inherit from parent.
    if let Some(ps) = parent_style {
        style.color = ps.color;
        style.font_size = ps.font_size;
        style.font_family = ps.font_family.clone();
        style.font_weight = ps.font_weight;
        style.letter_spacing = ps.letter_spacing;
        style.line_height = ps.line_height;
        style.text_align = ps.text_align;
        style.text_transform = ps.text_transform;
    }

    // Apply CSS rules with full ancestor-aware matching.
    {
        let node_ref = doc.get_node(node_id).unwrap();
        // Build a temporary PrdNode-like view for matching.
        for rule in rules {
            if compound_selector_matches(
                &rule.compound_selectors,
                node_ref,
                &html_id,
                ancestors,
            ) {
                // We can't mutate yet — collect declarations to apply.
                // But apply_property needs &mut style, so we match rule, store index.
                for (prop, val) in &rule.declarations {
                    apply_property(&mut style, prop, val, variables);
                }
            }
        }
    }

    // Inline styles (highest specificity).
    if !inline_style.is_empty() {
        for decl in inline_style.split(';') {
            let decl = decl.trim();
            if let Some((prop, val)) = decl.split_once(':') {
                apply_property(&mut style, prop.trim(), val.trim(), variables);
            }
        }
    }

    // ── 2b. Contain position:fixed inside stacking-context ancestors ──
    // Per CSS spec, an ancestor with `transform` establishes a containing
    // block for fixed-positioned descendants. In practice this means
    // position:fixed elements that are NOT direct children of <body> are
    // almost always contained by a transformed ancestor (e.g. .hud).
    // Our layout engine doesn't track containing blocks, so we approximate
    // this by downgrading position:fixed to position:absolute when the
    // element is deeper than a direct child of body (ancestors > 2 levels).
    if matches!(style.position, crate::prd::style::Position::Fixed) && ancestors.len() > 2 {
        style.position = crate::prd::style::Position::Absolute;
    }

    // ── 3. Write the new style back to the node ──────────────────────
    let finalized_style = style.clone();
    if let Some(node) = doc.get_node_mut(node_id) {
        node.style = style;
    }

    // ── 4. Build ancestor info and recurse into children ─────────────
    let mut child_ancestors = ancestors.to_vec();
    child_ancestors.push(AncestorInfo {
        tag: tag.clone(),
        classes: classes.clone(),
        html_id,
    });

    for cid in children {
        restyle_recursive(doc, cid, rules, variables, &child_ancestors, Some(&finalized_style));
    }
}

