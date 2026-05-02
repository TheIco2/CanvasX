// prism-runtime/src/devtools/debug_server.rs
//
// Minimal HTTP debug server that serves the current page's HTML/CSS
// in a standard browser for visual comparison. Runs on a background
// thread and binds to localhost.

use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}};
use std::thread;

/// State for the debug server.
pub struct DebugServer {
    /// Whether the server is currently running.
    running: Arc<AtomicBool>,
    /// The port the server is bound to (0 = not started).
    port: u16,
    /// Current page content to serve (html, css).
    content: Arc<Mutex<DebugContent>>,
}

struct DebugContent {
    html: String,
    css: String,
    title: String,
}

impl DebugServer {
    pub fn new() -> Self {
        Self {
            running: Arc::new(AtomicBool::new(false)),
            port: 0,
            content: Arc::new(Mutex::new(DebugContent {
                html: String::new(),
                css: String::new(),
                title: String::from("OpenRender Debug"),
            })),
        }
    }

    /// Whether the server is currently running.
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }

    /// The port the server is listening on.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// Update the served content from the active page source.
    pub fn update_content(&self, html_path: &PathBuf) {
        let html_source = match std::fs::read_to_string(html_path) {
            Ok(s) => s,
            Err(_) => return,
        };

        let css_path = html_path.with_extension("css");
        let css_source = if css_path.exists() {
            std::fs::read_to_string(&css_path).unwrap_or_default()
        } else {
            let sibling = html_path.parent()
                .map(|p| p.join("style.css"))
                .unwrap_or_default();
            if sibling.exists() {
                std::fs::read_to_string(sibling).unwrap_or_default()
            } else {
                String::new()
            }
        };

        let base_dir = html_path.parent();
        let (flattened_html, combined_css) =
            crate::compiler::html::flatten_html_for_debug(&html_source, &css_source, base_dir);

        let title = html_path.file_stem()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_else(|| "OpenRender Debug".to_string());

        if let Ok(mut content) = self.content.lock() {
            content.html = flattened_html;
            content.css = combined_css;
            content.title = title;
        }
    }

    /// Start the debug server on a background thread. Returns the port.
    pub fn start(&mut self) -> u16 {
        if self.running.load(Ordering::Relaxed) {
            return self.port;
        }

        // Bind to an available port.
        let listener = match TcpListener::bind("127.0.0.1:0") {
            Ok(l) => l,
            Err(e) => {
                log::error!("Debug server bind failed: {}", e);
                return 0;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        self.port = port;
        self.running.store(true, Ordering::Relaxed);

        let running = self.running.clone();
        let content = self.content.clone();

        // Set non-blocking so we can check the running flag periodically.
        let _ = listener.set_nonblocking(true);

        thread::spawn(move || {
            log::info!("Debug server started on http://127.0.0.1:{}", port);
            while running.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let content = content.clone();
                        // Handle each connection in a short-lived thread.
                        thread::spawn(move || {
                            let mut buf = [0u8; 4096];
                            let _ = stream.read(&mut buf);
                            let request = String::from_utf8_lossy(&buf);

                            let response = if request.starts_with("GET / ")
                                || request.starts_with("GET / HTTP")
                                || request.starts_with("GET /index")
                            {
                                build_html_response(&content)
                            } else if request.starts_with("GET /style.css") {
                                build_css_response(&content)
                            } else {
                                "HTTP/1.1 404 Not Found\r\nContent-Length: 9\r\n\r\nNot Found".to_string()
                            };

                            let _ = stream.write_all(response.as_bytes());
                            let _ = stream.flush();
                        });
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // No connection pending — sleep briefly.
                        thread::sleep(std::time::Duration::from_millis(50));
                    }
                    Err(_) => break,
                }
            }
            log::info!("Debug server stopped.");
        });

        port
    }

    /// Stop the debug server.
    pub fn stop(&mut self) {
        self.running.store(false, Ordering::Relaxed);
        self.port = 0;
    }
}

/// Build a full HTML page response.
fn build_html_response(content: &Arc<Mutex<DebugContent>>) -> String {
    let (html, _css, title) = match content.lock() {
        Ok(c) => (c.html.clone(), c.css.clone(), c.title.clone()),
        Err(_) => (String::new(), String::new(), "Error".to_string()),
    };

    // Strip <style> blocks from the flattened HTML (already captured in css).
    let body = strip_style_tags(&html);

    let page = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{title} — OpenRender Debug</title>
    <link rel="stylesheet" href="/style.css">
    <style>
        /* Reset to approximate OpenRender defaults */
        *, *::before, *::after {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{ background: #1a1a2e; color: #ffffff; font-family: 'Segoe UI', sans-serif; font-size: 14px; }}
    </style>
</head>
<body>
{body}
<script>
    // Handle data-navigate clicks (OpenRender SPA navigation).
    document.addEventListener('click', function(e) {{
        var target = e.target.closest('[data-navigate]');
        if (target) {{
            e.preventDefault();
            var page = target.getAttribute('data-navigate');
            console.log('[OpenRender Debug] Navigate to:', page);
            // Show/hide page-content sections.
            document.querySelectorAll('[data-page-id]').forEach(function(el) {{
                el.style.display = el.getAttribute('data-page-id') === page ? '' : 'none';
            }});
        }}
    }});
</script>
</body>
</html>"#,
        title = escape_html(&title),
        body = body,
    );

    let len = page.len();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        len, page,
    )
}

/// Build a CSS response.
fn build_css_response(content: &Arc<Mutex<DebugContent>>) -> String {
    let css = match content.lock() {
        Ok(c) => c.css.clone(),
        Err(_) => String::new(),
    };
    let len = css.len();
    format!(
        "HTTP/1.1 200 OK\r\nContent-Type: text/css; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        len, css,
    )
}

/// Strip <style>...</style> blocks from HTML.
fn strip_style_tags(html: &str) -> String {
    let mut result = String::with_capacity(html.len());
    let lower = html.to_lowercase();
    let mut pos = 0;
    while let Some(start) = lower[pos..].find("<style") {
        let abs_start = pos + start;
        result.push_str(&html[pos..abs_start]);
        if let Some(end) = lower[abs_start..].find("</style>") {
            pos = abs_start + end + 8; // skip past </style>
        } else {
            pos = html.len();
        }
    }
    result.push_str(&html[pos..]);
    result
}

/// Basic HTML entity escaping.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

