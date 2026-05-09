
# Quick Start Guide: PRISM

Welcome to PRISM! This guide will help you get started building a modern desktop app using PRISM, with real-world code and configuration examples.

## Prerequisites

- Rust toolchain ([rustup.rs](https://rustup.rs/))
- Git ([git-scm.com](https://git-scm.com/))
- (Optional) Visual Studio Code or your preferred IDE

## 1. Create a Minimal PRISM App

Start with a minimal `main.rs`:

```rust
use include_dir::{include_dir, Dir};

static PAGES: Dir<'_> = include_dir!("$CARGO_MANIFEST_DIR/pages");

fn main() {
  let app = prism_runtime::EmbeddedApp {
    prism_config_json: include_str!("../config.prism.json"),
    default_config_json: Some(include_str!("../config.default.json")),
    pages: &PAGES,
  };

  if let Err(e) = prism_runtime::run(app) {
    eprintln!("app error: {e}");
    std::process::exit(1);
  }
}
```

## 2. Add HTML UI Pages

Example `pages/base.html`:

```html
<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="UTF-8">
  <meta name="viewport" content="width=device-width, initial-scale=1.0">
  <title>My PRISM App</title>
  <link rel="stylesheet" href="assets/css/PrismFramework.css">
</head>
<body>
  <div class="prism-app">
    <include type="component" src="sidebar.html" /> <!-- <include> is a custom HTML component for PRISM -->
    <page-content default="home" /> <!-- <page-content> is a custom HTML component for PRISM -->
  </div>
  <div class="prism-toast-container" id="toast-container"></div>
</body>
<script src="assets/js/script.js"></script>
</html>
```

## 3. Example JavaScript for UI Logic

Example `assets/js/script.js`:

```js
function showToast(message, type) {
 type = type || 'info';
 var container = document.getElementById('toast-container');
 if (!container) return;
 var toast = document.createElement('div');
 toast.className = 'toast toast-' + type;
 toast.textContent = message;
 container.appendChild(toast);
 setTimeout(function() {
  toast.classList.add('toast-exit');
  setTimeout(function() {
   if (toast.parentNode) toast.parentNode.removeChild(toast);
  }, 300);
 }, 4000);
}
```

## 4. Example Configuration

Example `config.default.json`:

```json
{
 "theme": "Dark",
 "logging": {
  "level": "debug"
 }
}
```

## 5. Example Theme File

Example `themes/Dark.json`:

```json
{
 "name": "Dark",
 "variables": {
  "--bg":     "#111317",
  "--panel":  "#1a1d23",
  "--fg":     "#e9eaee",
  "--muted":  "#9aa1ac",
  "--accent": "#7ab8ff",
  "--border": "#2a2f38"
 },
 "css": ""
}
```

## 6. Example CSS for Theming

Example `assets/css/PrismFramework.css`:

```css
:root {
 --prism-accent: #8b6cff;
 --prism-bg-base: #0a0b14;
 --prism-bg-mica: #11131f;
 --prism-bg-layer-1: #161927;
 --prism-bg-layer-2: #1d2132;
 --prism-bg-layer-3: #262b3f;
}
body {
 background: var(--prism-bg-base);
 color: var(--fg, #e9eaee);
}
```

## 7. Project Structure Overview

- `src/`: Main Rust source code for your app
- `pages/`: HTML templates and static assets
  - `assets/`: CSS and JS for frontend
  - `components/`: components for frontend
  - `icons/`: Icons for frontend
  - `themes/`: Theme JSON files for UI customization

## 8. Building and Running

Build your app with:

```sh
cargo build --release
```

Run it with:

```sh
cargo run --release
```

---
This guide provides a foundation for building PRISM apps. Explore and customize further to create your own unique desktop experience!

## 7. Troubleshooting

- If you encounter build errors, ensure your Rust toolchain is up to date: `rustup update`
- Check that all dependencies are installed and paths are correct.

## 8. Learn More

- See the main PRISM README for advanced features and API documentation.
- Explore the code in `src/` and `pages/` to understand how to use PRISM.
- A full Documentation site through github pages is under developement

---
Happy coding with PRISM!
