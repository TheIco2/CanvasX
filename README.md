# OpenRender Runtime

A standalone GPU-native 2D scene graph renderer for desktop personalization, widgets, and interactive content. Written in Rust, powered by [wgpu](https://wgpu.rs/) (Vulkan / DX12). Part of the [OpenDesktop](https://github.com/The-Ico2/OpenDesktop) desktop customization platform.

> **Note:** This is one of my first Rust project, and I'm actively learning as I build. Expect rough edges and architectural evolution. If you spot bugs, design issues, or potential improvements, feel free to open a PR or reach out to me on **Discord** or **X (formerly Twitter)**.

---

## What It Does

OpenRender compiles a restricted subset of HTML and CSS into a binary intermediate format (**CXRD** — OpenRender Runtime Document), then renders it directly on the GPU via instanced SDF quads. No browser engine, no WebView, no JavaScript runtime — just a single Rust library that turns markup into pixels.

```bash
HTML/CSS → Compiler → CXRD Document → Layout → Animate → Paint → GPU Renderer
                                         ↑                           |
                                    IPC (live data)           wgpu (Vulkan/DX12)
```

The library crate (`OpenRender_runtime`) can be embedded into any Rust application. A standalone binary (`OpenRender-rt`) is also provided for quick prototyping and direct use.

---

## Features

- **GPU-native rendering** — Every UI element is a single instanced quad. The fragment shader evaluates SDF rounded rectangles with analytic anti-aliasing — no CPU tessellation or MSAA
- **HTML/CSS compiler** — Parses a useful subset of HTML (`div`, `span`, `section`, `p`, `h1`–`h6`, `img`, `button`, `input`, `data-bind`, `data-bar`, `data-bar-stack`, etc.) and CSS (selectors, flexbox, grid, variables, gradients, animations) into a fully resolved binary document
- **Layout engine** — Block flow, Flexbox (row/column, justify, align, grow/shrink/basis, gap, wrap), CSS Grid (`fr`/`auto`/`px`/`%`/`min-content`/`max-content`), absolute/fixed positioning
- **Animation runtime** — CSS `@keyframes` with duration, delay, iteration, direction, fill mode, and easing (linear, ease, ease-in/out, cubic-bezier). Supports 20+ animatable properties (opacity, transform, colors, dimensions, padding, margin, etc.)
- **Live data binding** — `<data-bind>` elements display live values from IPC (e.g. `binding="cpu.usage"`). `<data-bar>` and `<data-bar-stack>` render progress bars and multi-segment stacked bars
- **Text rendering** — GPU text via [glyphon](https://github.com/grovesNL/glyphon) (cosmic-text). Supports font families, weight, size, line-height, letter-spacing, text-transform, and text-align
- **CSS variables** — Custom properties (`--var`) resolved at compile time, with runtime updates via editable overrides
- **Image support** — PNG, JPEG, WebP embedded in the asset bundle. No network fetches
- **IPC bridge** — Generic named-pipe client for live data streaming. Optional `OpenDesktopBridge` for OpenDesktop-specific system data (CPU, GPU, RAM, storage, network, audio, etc.)
- **Platform layer** — Windows desktop embedding (WorkerW) for wallpapers, monitor enumeration with DPI awareness
- **Editable properties** — Runtime CSS variable overrides driven by `manifest.json` schema + `editable.yaml` user values
- **Compile-once caching** — CXRD documents are cached to disk with SHA-256 hash invalidation. No parsing during rendering

---

## Architecture

### Core Pipeline

Each frame follows the same path:

1. **Layout** — Resolve dimensions and positions for the entire CXRD tree (only when dirty)
2. **Animate** — Advance active animations and apply interpolated property values
3. **Update data** — Push latest IPC values into data-bound nodes and bars
4. **Prepare text** — Shape and layout all text buffers via glyphon
5. **Paint** — Depth-first traversal of the node tree → flat `Vec<UiInstance>` for the GPU
6. **Render** — Submit instanced draw call + text pass to wgpu

### Module Overview

| Module | Purpose |
| -------- | --------- |
| `compiler/` | HTML/CSS → CXRD compilation, asset bundling, editable property bridging |
| `cxrd/` | Binary scene graph format: nodes, styles, animations, assets, input types |
| `gpu/` | wgpu context, render pipeline, SDF shader, texture manager, instanced rendering |
| `layout/` | Block flow, Flexbox, CSS Grid, absolute/fixed positioning |
| `scene/` | Scene graph coordinator, paint pass, text painter, input handler, app host |
| `animate/` | Animation timeline, keyframe interpolation, easing functions |
| `ipc/` | Named-pipe client, protocol types, OpenDesktop bridge (optional) |
| `platform/` | Monitor enumeration, WorkerW desktop embedding (Windows) |

### GPU Rendering

All rendering is instanced — one quad per UI element. The WGSL fragment shader (`ui_box.wgsl`) evaluates:

- SDF rounded rectangles with per-corner radius
- Per-side border widths and colors
- Background colors, gradients, and texture sampling
- Clip rectangle masking
- Per-instance opacity
- Analytic anti-aliasing (no MSAA)

Global uniforms provide viewport size, elapsed time, and DPI scale factor.

### Distribution Formats

| Format | Extension | Description |
| -------- | ----------- | ------------- |
| **CXRD** | `.cxrd` | Single compiled document (binary, serde + bincode) |
| **CXRP** | `.cxrp` | Package — ZIP archive of multiple CXRD documents + shared assets |
| **CXRL** | `.cxrl` | Library — reusable component subtrees, shared styles, animation presets |

---

## Supported HTML Elements

| Element | Behavior |
| --------- | ---------- |
| `div`, `section` | Container (block or flex/grid depending on CSS) |
| `span`, `p`, `h1`–`h6`, `label` | Text node |
| `img` | Image from asset bundle (fit: fill, contain, cover, none) |
| `button` | Interactive button (Primary, Secondary, Danger, Ghost, Link variants) |
| `input` | Text input, textarea, checkbox/toggle, slider, dropdown, color picker |
| `svg` / `path` | SVG path rendering (d, stroke, fill) |
| `data-bind` | Live data display (binding path + optional format string) |
| `data-bar` | Single progress bar (binding + max + color) |
| `data-bar-stack` | Multi-segment stacked bar (shared max-binding, per-segment binding + color) |

---

## Supported CSS Properties

<details>
<summary>Full property list (40+)</summary>

**Box Model:** `display`, `position`, `overflow`, `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height`, `margin`, `padding`, `box-sizing`

**Flexbox:** `flex-direction`, `flex-wrap`, `justify-content`, `align-items`, `align-self`, `flex-grow`, `flex-shrink`, `flex-basis`, `gap`, `row-gap`, `column-gap`

**Grid:** `grid-template-columns`, `grid-template-rows`, `grid-column`, `grid-row`, `grid-column-start`, `grid-column-end`, `grid-row-start`, `grid-row-end`

**Position:** `top`, `right`, `bottom`, `left`, `z-index`

**Background:** `background`, `background-color`, `background-image` (linear-gradient, radial-gradient)

**Border:** `border`, `border-color`, `border-width`, `border-radius`, `border-top`/`right`/`bottom`/`left`

**Typography:** `color`, `font-family`, `font-size`, `font-weight`, `line-height`, `text-align`, `letter-spacing`, `text-transform`

**Visual:** `opacity`, `box-shadow`

**Units:** `px`, `%`, `rem`, `em`, `vw`, `vh`, `auto`, `fr` (grid only)

**Selectors:** Tag, `.class`, `#id`, descendant combinator, compound selectors

**Other:** CSS custom properties (`--var` / `var()`), `@keyframes`

</details>

---

## Usage

### As a Library

```toml
[dependencies]
OpenRender-runtime = { path = "../OpenRender" }
```

```rust
use OpenRender_runtime::{GpuContext, SceneGraph};
use OpenRender_runtime::compiler::html::compile_html;
use OpenRender_runtime::gpu::renderer::Renderer;
use OpenRender_runtime::cxrd::document::SceneType;

// Compile HTML/CSS to CXRD
let doc = compile_html(&html, &css, "my-scene", SceneType::Wallpaper, Some(&asset_dir));

// Create GPU context from a window handle
let gpu_ctx = GpuContext::from_raw_hwnd(hwnd, width, height);
let mut renderer = Renderer::new(&gpu_ctx);
let mut scene = SceneGraph::new(doc);

// Per-frame render loop
let (instances, clear_color) = scene.tick(vw, vh, dt, &mut renderer.font_system);
renderer.render(&gpu_ctx, &instances, clear_color, scale);
```

### As a Standalone Binary

```bash
OpenRender-rt --wallpaper --source index.html --css style.css --monitor 0
OpenRender-rt --widget --source panel.html --fps 60
OpenRender-rt --config --source settings.html
```

| Flag | Description |
| ------ | ------------- |
| `--wallpaper` | Embed into desktop (WorkerW) |
| `--statusbar` | Status bar mode |
| `--widget` | Floating widget window |
| `--config` | Configuration panel |
| `--source` | Path to HTML file |
| `--css` | Path to CSS file (optional, defaults to companion `.css`) |
| `--monitor` | Monitor index (default: primary) |
| `--fps` | Target frame rate |

---

## IPC Data Binding

OpenRender nodes can bind to live data via the IPC bridge. Data keys use dot-notation paths (e.g. `cpu.usage`, `ram.used_gb`, `storage.disks.0.used_bytes`).

```html
<data-bind binding="cpu.usage" format="{value}%"></data-bind>
<data-bar binding="ram.used_bytes" max-binding="ram.total_bytes" style="..."></data-bar>
<data-bar-stack max-binding="storage.total_bytes">
    <data-bar-segment binding="storage.disks.0.used_bytes" style="background: var(--disk0-color)"></data-bar-segment>
    <data-bar-segment binding="storage.disks.1.used_bytes" style="background: var(--disk1-color)"></data-bar-segment>
</data-bar-stack>
```

When connected to OpenDesktop, the `OpenDesktopBridge` polls 16 data sections (time, cpu, gpu, ram, storage, displays, network, wifi, bluetooth, audio, keyboard, mouse, power, idle, system, processes) and flattens them into a key-value map consumed by the scene graph.

---

## Dependencies

| Category | Crate | Version |
| ---------- | ------- | --------- |
| GPU | `wgpu` | 28.0 |
| Windowing | `winit` | 0.30 |
| Text | `glyphon` | 0.10 |
| GPU buffers | `bytemuck` | 1.25 |
| Images | `image` | 0.25 |
| Serialization | `serde`, `serde_json`, `bincode` | 1.0 / 1.0 / 2.0 |
| CSS parsing | `cssparser` | 0.36 |
| Platform | `windows` | 0.62 |
| File watching | `notify` | 8.2 |
| Directory walking | `walkdir` | 2.5 |
| Archives | `zip` | 8.1 |
| Hashing | `sha2`, `hex` | 0.10 / 0.4 |
| Concurrency | `parking_lot` | 0.12 |
| Collections | `smallvec`, `rustc-hash` | 1.15 / 2.1 |

---

## Requirements

- Windows 10/11
- GPU with Vulkan or DirectX 12 support
- OpenDesktop Backend (`OpenDesktop.exe`) — only required for live data binding

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

## Project Status

Under active development (`v0.1.0`). APIs, document format, and behavior may change.

> Currently this project is being developed for OpenDesktop, but in the future I intend to completely decouple this from OpenDesktop and make it its own thing for everyone to use if they wish.

---

## Contact

- **Discord:** the_ico2
- **X (Twitter):** The_Ico2
