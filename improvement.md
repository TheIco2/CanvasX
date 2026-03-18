# CanvasX — Comprehensive Code Analysis & Improvement Plan

> **Generated from a full read-through of every source file in the repository.**
> Crate: `canvasx-runtime` (binary: `canvasx-rt`)  |  License file: Apache 2.0  |  Cargo.toml claims MIT

---

## Table of Contents

1. [Architecture Overview](#1-architecture-overview)
2. [Per-Module Analysis](#2-per-module-analysis)
3. [Specific Questions Answered](#3-specific-questions-answered)
4. [Cross-Cutting Issues](#4-cross-cutting-issues)
5. [Improvement Roadmap](#5-improvement-roadmap)

---

## 1. Architecture Overview

```ps1
HTML/CSS/JS source files
        │
        ▼
  ┌──────────────┐     ┌────────────┐
  │  Compiler    │────▶│   CXRD     │  (document format — JSON inside magic-header envelope)
  │  html / css  │     │  Document  │
  │  bundle      │     └─────┬──────┘
  └──────────────┘           │
                             ▼
  ┌──────────────┐   ┌──────────────┐   ┌─────────────┐
  │  Layout      │──▶│  Animate     │──▶│  Paint      │  (scene/paint.rs → UiInstance quads)
  │  engine.rs   │   │  timeline.rs │   │  scene/*    │
  └──────────────┘   └──────────────┘   └──────┬──────┘
                                               │
                             ┌─────────────────┘
                             ▼
  ┌──────────────┐   ┌──────────────┐
  │  GPU         │──▶│  wgpu        │  (Vulkan / DX12 via SDF rounded-rect shader)
  │  renderer.rs │   │  surface     │
  └──────────────┘   └──────────────┘
        ▲                    ▲
        │                    │
  ┌──────────────┐   ┌──────────────┐
  │  Scripting   │   │  IPC         │
  │  V8 + Canvas │   │  Named pipes │
  │  tiny-skia   │   │  JSON proto  │
  └──────────────┘   └──────────────┘
```

**Key design choices:**

- Single GPU pipeline with SDF rounded rectangles (instanced quads).
- Layout is fully custom (block + flexbox + CSS Grid subset).
- V8 runs on a dedicated thread (`js_worker`), communicates via `mpsc` channels.
- Canvas 2D is software-rendered by `tiny-skia`, then texture-uploaded to wgpu.
- IPC is Windows named-pipe based (generic protocol + Sentinel-specific bridge).
- Distribution formats: `.cxrd` (single doc), `.cxrl` (component library), `.cxrp` (package).
- Platform: **Windows-only** (Win32 WorkerW embedding, named pipes, DPI via HiDpi API).

---

## 2. Per-Module Analysis

### 2.1 Root Files

#### `Cargo.toml`

- **Summary:** Workspace member, 30+ dependencies. Release profile: opt-level 3, thin LTO, strip.
- **Issues:**
  - `cssparser = "0.36"` listed but **never imported** — the project uses a hand-rolled CSS parser in `compiler/css.rs`.
  - `bincode = "2"` listed but CXRD serialisation actually uses `serde_json`.
  - `license = "MIT"` contradicts the `LICENSE` file which is Apache 2.0.
- **Recommendation:** Remove unused deps (`cssparser`, `bincode`). Fix license field.

#### `src/lib.rs`

- Module declarations + re-exports. Clean.

#### `src/main.rs` (~500 lines)

- `CliArgs` struct parses command-line. `App` struct holds all runtime state.
- Implements `winit::application::ApplicationHandler`.
- Issues: uses `log::warn!` for non-warning informational messages in many places.

#### `src/logging.rs`

- Custom async file logger → `~/.Sentinel/logs/CanvasX.log`.
- Background writer via `mpsc`. Panics on double-init.
- **Issue:** No log rotation; file grows without bound.

#### `build.rs`

- Only sets Windows icon. Fine.

---

### 2.2 GPU Module (`src/gpu/`)

#### `context.rs` — `GpuContext`

- Creates wgpu Instance/Adapter/Device/Queue/Surface.
- Has `new()` (from winit window) and `from_raw_hwnd()` (for WorkerW embedding).
- **Issues:**
  - `desired_maximum_frame_latency` is **3** in `new()` but **1** in `from_raw_hwnd()` — inconsistent.
  - `present_mode` is `Fifo` (VSync) in normal path, `Mailbox` in raw_hwnd — undocumented difference.
  - Explicitly picks non-sRGB surface format. Comment explains why, but this means the shader must handle gamma manually (it doesn't — colours will appear slightly incorrect on sRGB monitors).

#### `pipeline.rs` — `UiPipelines`

- Single pipeline: vertex (unit quad → pixel → NDC) + fragment (SDF rounded rect).
- `GlobalUniforms`: viewport, time, scale.
- **Issue:** Only one texture bound at a time (not a texture array). Every texture switch = a new draw call. This limits batching.

#### `renderer.rs` — `Renderer`

- Instance buffer starts at 1024 quads, doubles on demand.
- Batches draw calls by `texture_index`.
- Uploads canvas textures, manages text rendering via glyphon.
- **Issues:**
  - Gradient textures are generated every frame in `paint.rs` but the bind groups for them are managed ad-hoc — potential orphaned bind groups.
  - Text atlas trimmed only every 64 frames — could leak if many unique text strings appear.

#### `texture.rs` — `TextureManager`

- Manages GPU textures: load from bytes, update streaming, default 1×1 texture.
- `update_texture()` smartly reuses same-size textures via `queue.write_texture`.
- Clean and well-structured.

#### `vertex.rs` — `UiInstance`

- 96-byte instance with rect, colours, border, clip, texture index, opacity, flags.
- Flags: `HAS_BACKGROUND`, `HAS_BORDER`, `HAS_TEXTURE`, `HAS_CLIP`.

#### `shaders/ui_box.wgsl`

- SDF outer + inner for border ring. Anti-aliased via `fwidth + smoothstep`.
- Canvas textures (index 10000–19999) get **1.14× luminance lift** — hard-coded hack.
- **Issue:** The luminance lift is a band-aid for gamma issues caused by using non-sRGB surface format. A proper sRGB workflow would eliminate this.

---

### 2.3 Layout Module (`src/layout/`)

#### `engine.rs` (~800 lines) — `compute_layout()`

- Supports Block, Flex (inline), and CSS Grid (4-phase: placement → track sizing → offsets → positioning).
- Grid supports: `fr`/`px`/`%`/`auto` tracks, negative grid lines, `span`, auto-placement.
- `apply_scale_to_subtree()` for `transform: scale()`.
- **Issues:**
  - `MinContent` / `MaxContent` grid tracks treated **same as Auto** — incorrect.
  - Auto columns have minimal support (treated as 0 width).
  - No `flex-wrap` logic despite `FlexWrap` enum being defined.
  - `estimate_content_height/width()` is rudimentary — doesn't account for line wrapping.

#### `flex.rs` (~250 lines) — `layout_flex()`

- Standalone function implementing flex layout.
- **Issue:** This function appears **unused** — `engine.rs` has its own inline flex implementation (`layout_flex_children()`). Dead code.

#### `types.rs`

- `LayoutConstraints`, `LayoutBox`. Clean.

---

### 2.4 Scene Module (`src/scene/`)

#### `graph.rs` — `SceneGraph`

- Coordinates the tick pipeline: layout → animate → text → paint.
- Manages `data_values` HashMap for IPC-bound data.
- `apply_custom_data_tags()`: processes `data-bind` (text replacement) and `data-bar` (width as percentage).
- Dirty flags: `layout_dirty`, `paint_dirty`, `gradient_textures_dirty`, `data_bound_dirty`.

#### `paint.rs` (~550 lines) — `paint_document()`

- Depth-first tree traversal → `UiInstance` quads.
- Handles: box-shadow, linear/radial gradients, images, borders, clip rects, z-index sorting.
- `paint_input_widget()`: emits visual chrome for 13+ input widget types (Button, TextInput, Checkbox, Slider, Dropdown, ColorPicker, TabBar, ScrollView, Link, AssetSelector, etc.).
- **Issues:**
  - Box shadows use a **simplified blur approximation** (layered semi-transparent shells) — not actual Gaussian blur.
  - Gradient textures are **re-rasterized every frame** (capped at 512×512). No caching.
  - `apply_backdrop_fallback()` approximates `backdrop-filter: blur()` by adjusting alpha/luminance — very rough.
  - Gradient slot counter uses `AtomicU32` starting at 20000, reset each frame — functional but fragile.

#### `text.rs` — `TextPainter`

- Buffer cache keyed by content hash + style + dimensions. Uses glyphon.
- `format_data_value()` with `{bytes}`, `{speed}`, `{uptime}`, `{.N}` tokens.
- **Issue:** Marked `#[allow(dead_code)]` — `format_data_value()` is not called from `graph.rs`. Likely intended feature that was disconnected.

#### `input_handler.rs` — `InputHandler`

- Per-node `InteractionState` with hover/focus tracking, hit testing.
- Maps winit input events → `UiEvent` (Click, ValueChanged, CheckChanged, SliderChanged, etc.).
- `dispatch_click()` processes `EventBinding` actions.
- **Issues:**
  - Hit-testing traverses the entire tree every mouse event — O(n) per event.
  - `CursorIcon` enum defined but **cursor changes are not wired** to the platform layer.

#### `app_host.rs` (~500 lines) — `AppHost`

- Multi-page navigation with routes, history/forward stacks, protocol handlers.
- `SentinelAppBuilder`: constructs sidebar+content layout for Sentinel applications.
- `PageSource` enum: Document, HtmlFile, Inline, ProtocolUri, External.

---

### 2.5 Compiler Module (`src/compiler/`)

#### `html.rs` (~500+ lines) — `compile_html()`

- Custom HTML tokenizer + tree builder. Collects `<script>` blocks via thread-local `COLLECTED_SCRIPTS`.
- CSS rule matching with specificity-like ordering.
- `propagate_inherited_styles()`: handles color, font-family, font-size, font-weight, line-height, letter-spacing, text-align.
- `restyle_document()`: public API for restyling after DOM changes.
- **Issues:**
  - `COLLECTED_SCRIPTS` uses `thread_local!` — not safe if compilation is ever called from multiple threads.
  - No HTML entity decoding (`&amp;`, `&lt;`, etc.).
  - No support for self-closing tags like `<br/>` during compilation (only during `setInnerHTML`).

#### `css.rs` (~800+ lines) — `parse_css()`

- Hand-written character-by-character CSS parser.
- Massive `apply_property()` match handles 50+ CSS properties.
- Supports: `calc()` (basic arithmetic), `var()` references, gradients, `rem`/`em`/`vw`/`vh`.
- **Issues:**
  - `@keyframes` parsing is **TODO** — animations must be defined in CXRD directly.
  - CSS `transition` parsing is commented out / TODO.
  - Pseudo-elements return empty (unmatchable) selectors — pseudo-classes not supported.
  - No `!important` support.
  - No shorthand property expansion (e.g., `margin: 10px 20px` works, but `border: 1px solid red` is parsed as border-width only).

#### `bundle.rs` — `bundle_assets()`

- Walks directory, loads images + fonts. `guess_weight_from_name()` for font weight detection.
- Clean.

#### `editable.rs` — `EditableContext`

- Bridges `manifest.json` editable schema + `editable.yaml` overrides.
- Custom minimal YAML-to-JSON parser (avoids `serde_yaml` dependency).
- **Issue:** The YAML parser is very basic — may break on nested structures or multiline strings.

---

### 2.6 CXRD Module (`src/cxrd/`) — Document Format

#### `document.rs` — `CxrdDocument`

- Binary format: magic `"CXR\x01"` + version u32 + body_len u64 + JSON body.
- **Critical Issue:** Despite the "binary format" naming and `bincode` dependency, the body is actually **serialised as JSON** via `serde_json`. The `bincode` crate is **unused**. This means "binary" CXRD files are just JSON with a small header — much larger than necessary.

#### `node.rs` — `CxrdNode`

- `NodeKind`: Container, Text, Image, SvgPath, Canvas, ScrollContainer, Input(InputKind).
- `EventBinding` + `EventAction`: ToggleClass, SetClass, RemoveClass, Navigate, IpcCommand, StartAnimation, ScrollTo.

#### `style.rs` — `ComputedStyle`

- Comprehensive: ~40+ fields, all enums for Display, Position, Overflow, etc.
- `TransitionDef` and `EasingFunction` defined but transitions aren't wired up.

#### `value.rs` — Dimension, Color, Rect, EdgeInsets, CornerRadii

- `Dimension::resolve()` resolves px/percent/rem/em/vw/vh/auto against parent + viewport.
- `Color::lerp()` for animation interpolation.

#### `animation.rs` — AnimationDef, Keyframe, AnimatableProperty

- 21 animatable property variants. Well-designed.

#### `asset.rs` — AssetBundle

- Images + Fonts + Data assets. Custom `serde_bytes_compat` module.

#### `input.rs` — InputKind, InteractionState

- 11 input types with runtime focus/interaction state.
- Clean.

---

### 2.7 CXRL Module (`src/cxrl/`) — Library Format

- **ZIP-based** archive containing: `manifest.json` + components + themes + animation presets + assets.
- `LibraryBuilder`: fluent API for constructing `.cxrl` archives. SHA-256 integrity hashes.
- `LoadedLibrary`: reads ZIP, extracts entries by ID.
- `LibraryManifest`: rich metadata (id, version, author, tags, dependencies, min runtime version).
- Well-structured, production-quality.

---

### 2.8 CXRP Module (`src/cxrp/`) — Package Format

- **ZIP-based** archive containing: `manifest.json` + `.cxrd` documents + assets + library refs.
- `PackageBuilder`: fluent API. Categorises assets by MIME type.
- `LoadedPackage`: reads ZIP, deserialises embedded CXRD documents.
- `PerformanceHints`: target FPS, GPU usage level, layer count.
- Well-structured, production-quality.

---

### 2.9 IPC Module (`src/ipc/`)

#### `protocol.rs`

- Wire format: `IpcRequest { ns, cmd, args }` → `IpcResponse { ok, data, error }`. JSON-based.

#### `client.rs` — `IpcClient`

- Generic named-pipe client. Background poller thread.
- `send_ipc_request_to()`: unsafe Win32 `CreateFileW`/`ReadFile`/`WriteFile` with message mode.
- `flatten_json_to_map()`: nested JSON → dotted key paths.
- **Issues:**
  - `Mutex::lock().unwrap()` throughout — will panic on poisoned mutex.
  - Background poll loop runs forever with no shutdown mechanism.
  - No reconnection backoff — retries immediately on failure.

#### `sentinel.rs` — `SentinelBridge`

- Sentinel-specific high-level IPC client.
- Tracking demands, heartbeats, addon management, backend control.
- Reconnection with 1s delay on disconnect. Re-sends demands on reconnect.
- Well-designed for its purpose.

---

### 2.10 Scripting Module (`src/scripting/`)

#### `v8_runtime.rs` (~1900 lines) — `JsRuntime`

- V8 isolate with full DOM shim. Registers 40+ native functions (`__cx_*`).
- `JS_SHIM` (~400 lines): JavaScript polyfill providing `document.getElementById`, `querySelector`, `createElement`, `requestAnimationFrame`, Canvas 2D context factory, `setTimeout`/`setInterval`, `Sentinel.subscribe`, `fetch` (sync IPC-based), etc.
- `set_inner_html()`: full HTML parser embedded inside the runtime for dynamic DOM manipulation.
- **Issues:**
  - `RUNTIME_STATE` uses `thread_local! + RefCell` — panics if re-entered.
  - Unsafe pointer casts from `&st.css_variables` / `&st.css_rules` to raw pointers for split borrows — correct but fragile. Would be cleaner with `RefCell` splitting.
  - `isDescendant()` in JS shim **always returns true** — `querySelector` on elements doesn't actually scope to descendants.
  - `performance.now()` returns wall-clock time instead of monotonic high-resolution time.
  - `cx_ipc_send` is **synchronous/blocking** inside V8 — blocks the JS thread during IPC calls.
  - `selector_matches_node()` only handles simple selectors (`.class`, `#id`, `tag`) — no compound selectors in JS-side querySelector.
  - `get_computed_style_value()` only handles a small subset of properties (display, opacity, font-size, color, width, height).

#### `canvas2d.rs` (~1050 lines) — `CanvasBuffer`, `CanvasManager`

- Full Canvas 2D state machine backed by `tiny-skia`.
- Supports: fill/stroke rect/path, gradients (linear/radial), patterns, transforms, save/restore, blend modes, arc approximation.
- `parse_css_color()`: handles hex, rgb, rgba, hsl, hsla, named colours.
- **Issues:**
  - `fill_text()` is a **no-op** — Canvas 2D text rendering not implemented.
  - `arc()` approximates with line segments — visible straight-line artifacts on large arcs.
  - Only 6 named colours supported (black, white, red, green, blue, transparent).
  - `clip()` is not connected — `clip_path` field exists but never used in fill/stroke operations.

#### `js_worker.rs` — `JsWorkerHandle`

- Offloads V8 to a background thread. `JsCommand`/`JsResult` via `mpsc`.
- Drains stale tick commands (only latest tick executes).
- **Issue:** `wait_for_init()` panics if JS worker crashes during init — should return `Result`.

---

### 2.11 Animate Module (`src/animate/`)

#### `timeline.rs` — `AnimationTimeline`

- Manages `ActiveAnimation` instances. Advances each frame.
- Handles: delay, iteration count (including infinite), direction (normal/reverse/alternate), fill modes.
- Well-implemented.

#### `interpolate.rs`

- Property-by-property `lerp` for all 21 `AnimatableProperty` variants.
- `apply_animated_property()` maps interpolated values back to `ComputedStyle`.
- **Issue:** Transform properties (TranslateX/Y, ScaleX/Y, Rotate) are interpolated but `apply_animated_property()` does nothing with them (`_ => {}`). They're supposed to be applied via transform matrix on the instance, but this isn't wired up.

#### `easing.rs`

- Cubic Bézier evaluation via Newton's method (8 iterations). Standard CSS easing functions.
- Has unit tests. Clean.

---

### 2.12 Platform Module (`src/platform/`)

#### `desktop.rs` — WorkerW embedding

- Finds desktop's WorkerW layer via Progman message `0x052C`.
- `embed_in_desktop()`: parents the render window into WorkerW for wallpapers.
- Windows-only. Works.

#### `monitor.rs` — `enumerate_monitors()`

- DPI-aware monitor enumeration via Win32 `EnumDisplayMonitors` + `GetDpiForMonitor`.
- Returns `MonitorInfo` with position, size, scale factor, primary flag.
- Clean.

---

## 3. Specific Questions Answered

### Q1: What rendering backend does it use? Is it DirectX, OpenGL, or Vulkan?

**wgpu 28.0.0** with backends explicitly set to `VULKAN | DX12`. Despite the README mentioning DX11 fallback, the code **does not enable DX11** (`wgpu::Backends::DX11` is not included). OpenGL is also not enabled. The rendering is done via a single SDF-based instanced-quad pipeline in WGSL.

### Q2: How does the HTML/CSS/JS compilation pipeline actually work?

1. **CSS parsing** (`compiler/css.rs`): Hand-written character-by-character parser extracts `CssRule` objects with selectors and declarations. Supports `calc()`, `var()`, gradients.
2. **HTML parsing** (`compiler/html.rs`): Custom tokenizer builds a `CxrdDocument` node tree. For each node, CSS rules are matched (compound selector matching with descendant combinators) and declarations applied via `apply_property()`.
3. **CSS inheritance** (`propagate_inherited_styles`): Inheritable properties propagated parent→child.
4. **Asset bundling** (`compiler/bundle.rs`): Images and fonts are loaded and indexed.
5. **Script collection**: `<script>` tags collected during HTML parsing. Executed later by V8.
6. **Result**: `CxrdDocument` + `Vec<ScriptBlock>` + `Vec<CssRule>`.

### Q3: How does the IPC system work?

Windows named pipes with JSON wire protocol:

- **Request**: `{ ns: string, cmd: string, args?: any }`
- **Response**: `{ ok: bool, data?: any, error?: string }`
- **Generic client** (`ipc/client.rs`): Configurable pipe name, poll interval, poll request.
- **Sentinel bridge** (`ipc/sentinel.rs`): Sentinel-specific high-level client with tracking demands, heartbeats, addon/registry management.
- **JS bridge**: V8's `__cx_ipc_send()` sends synchronous IPC requests to any pipe.
- Background polling thread fetches data snapshots and flattens them into `HashMap<String, String>` for data-binding.

### Q4: What are the CXRD, CXRL, and CXRP formats?

| Format | Extension | Container | Purpose |
| -------- | ----------- | ----------- | --------- |
| CXRD | `.cxrd` | Magic header + JSON body | Single document (scene/page) |
| CXRL | `.cxrl` | ZIP archive | Reusable component library (components, themes, animation presets, assets) |
| CXRP | `.cxrp` | ZIP archive | Distribution package (multiple documents + shared assets + library refs) |

**CXRD** is NOT truly binary — it wraps JSON in a `"CXR\x01"` + version + length header. The `bincode` dependency is listed but unused.

### Q5: What's the maturity level?

**Early-to-mid prototype.** The core rendering pipeline works (layout → paint → GPU), V8 integration is functional with a DOM shim, IPC works for live data binding. However:

- Many CSS features are TODO (@keyframes parsing, transitions, pseudo-classes).
- Canvas 2D text is unimplemented.
- Flex-wrap is unimplemented.
- Grid MinContent/MaxContent is wrong.
- Transform animations don't apply to rendering.
- Several dead-code paths.
- Windows-only.

### Q6: V8 / JavaScript runtime integration status?

**Functional.** V8 runs on a dedicated thread with full DOM-like API:

- `document.getElementById/querySelector/createElement/appendChild` — all work
- `requestAnimationFrame` — works
- `setTimeout/setInterval` — implemented via polling in rAF tick
- Canvas 2D — works (except `fillText`)
- `Sentinel.subscribe` — data binding from IPC to JS
- `fetch` — synchronous IPC-based (not HTTP)
- Style manipulation: `el.style.X = v` works
- DOM mutations: `innerHTML`, `classList`, `createElement`, `removeChild` — all work

### Q7: Security concerns?

1. **V8 sandbox**: No V8 sandbox/snapshot is configured. Scripts have full access to IPC (can send to any named pipe). No CSP-like restrictions.
2. **IPC**: Named pipes are world-accessible by default on Windows. No authentication or authorization on IPC requests.
3. **Unsafe code**: Several `unsafe` blocks in `ipc/client.rs` (Win32 FFI), `v8_runtime.rs` (raw pointer casts for split borrows), `platform/desktop.rs` (Win32 window manipulation). All appear correct but lack safety comments.
4. **File I/O**: `execute_file()` reads arbitrary paths from `<script src="...">`. No sandboxing of file access.
5. **ZIP archives**: `.cxrl`/`.cxrp` loaders use the `zip` crate. No zip-bomb protection, no path traversal checks on archive entry names.
6. **Base64 decoding**: Custom implementation in `v8_runtime.rs` — should use a well-tested crate instead.

### Q8: Performance concerns?

1. **Gradient textures re-rasterized every frame** — should cache when unchanged.
2. **One texture binding per draw call** — texture array or atlas would batch better.
3. **Hit-testing is O(n) per mouse event** — needs spatial index for complex scenes.
4. **IPC polling thread per bridge** — fine for one bridge, doesn't scale.
5. **Canvas pixmap cloned for `drawImage`/`fill_rect_with_pattern`** — avoids borrow issues but allocates.
6. **`isDescendant()` always returns true** — `querySelector` on elements scans entire document.
7. **`Mutex::lock().unwrap()` everywhere** — panics on poisoned mutexes instead of recovering.
8. **No frame rate limiting beyond VSync** — CPU spins on event loop.

### Q9: Missing error handling?

1. **`unwrap()` on mutex locks** throughout `ipc/client.rs` and `ipc/sentinel.rs`.
2. **`expect()` on thread spawn** in `js_worker.rs` — will panic if thread limit reached.
3. **`panic!` in `wait_for_init()`** if JS worker sends unexpected message.
4. **`logging.rs` panics** if `init()` called twice.
5. **Shader compilation** — no fallback if WGSL fails to compile.
6. **V8 initialisation** — no error path if V8 platform creation fails.
7. **Surface configuration** — no handling of `SurfaceError::Lost` beyond logging.
8. Many `let _ =` discarded results in `ipc/client.rs` (close handle, set pipe mode).

---

## 4. Cross-Cutting Issues

### 4.1 Dead Code & Unused Dependencies

| Item | Location | Issue |
| ------ | ---------- | ------- |
| `cssparser` crate | `Cargo.toml` | Listed but never imported |
| `bincode` crate | `Cargo.toml` | Listed but never used (CXRD is JSON) |
| `layout_flex()` | `layout/flex.rs` | Standalone flex function, duplicated by `engine.rs` inline flex |
| `format_data_value()` | `scene/text.rs` | `#[allow(dead_code)]` — never called |
| `TransitionDef` | `cxrd/style.rs` | Defined but transitions not implemented |

### 4.2 Inconsistencies

| Item | Detail |
| ------ | -------- |
| License | `Cargo.toml` says MIT; `LICENSE` file is Apache 2.0 |
| present_mode | `Fifo` in normal path, `Mailbox` in raw_hwnd |
| desired_maximum_frame_latency | 3 in normal, 1 in raw_hwnd |
| CXRD format | Called "binary" but uses JSON internally |
| README DX11 claim | Code only enables Vulkan + DX12 |

### 4.3 Thread Safety

- `COLLECTED_SCRIPTS` in `compiler/html.rs` uses `thread_local!` — won't work if compilation is multi-threaded.
- `RUNTIME_STATE` in `v8_runtime.rs` uses `thread_local! + RefCell` — panics on re-entry.
- `ipc/sentinel.rs` uses `Arc<Mutex<>>` properly, but all locks are `.unwrap()`.

---

## 5. Improvement Roadmap

### Priority 1: Correctness & Stability

- [ ] **Fix license mismatch** — decide MIT or Apache 2.0, update both `Cargo.toml` and `LICENSE`.
- [ ] **Remove unused crate dependencies** — `cssparser`, `bincode`.
- [ ] **Remove dead code** — unused `layout/flex.rs`, dead `format_data_value()`.
- [ ] **Fix `isDescendant()` in JS shim** — currently always returns `true`.
- [ ] **Wire up transform animations** — `apply_animated_property()` currently ignores TranslateX/Y, ScaleX/Y, Rotate.
- [ ] **Fix MinContent/MaxContent grid track sizing** — currently treated as Auto.
- [ ] **Replace `unwrap()` on mutex locks** with `.lock().unwrap_or_else()` or proper error propagation.
- [ ] **Add `zip` path traversal checks** in CXRL/CXRP loaders.

### Priority 2: Feature Completion

- [ ] **Implement CSS `@keyframes` parsing** in `compiler/css.rs`.
- [ ] **Implement CSS transitions** (wire up `TransitionDef` to `AnimationTimeline`).
- [ ] **Implement `flex-wrap`** in the layout engine.
- [ ] **Implement Canvas 2D `fillText()`** via cosmic-text → tiny-skia.
- [ ] **Implement proper arc drawing** in Canvas 2D (use tiny-skia cubic decomposition instead of line segments).
- [ ] **Expand `get_computed_style_value()`** to cover all CSS properties.
- [ ] **Implement scrollbar rendering** for ScrollView.
- [ ] **Wire cursor icon changes** to the platform layer.

### Priority 3: Performance

- [ ] **Cache gradient textures** — only re-rasterize when gradient parameters change.
- [ ] **Texture array or atlas** — reduce draw calls from texture-switch batching.
- [ ] **Spatial index for hit-testing** — replace O(n) tree scan with accelerated lookup.
- [ ] **Avoid cloning pixmaps** in `drawImage`/`fill_rect_with_pattern` — use index-based split borrows.
- [ ] **Implement proper sRGB workflow** — use sRGB surface format, remove 1.14× luminance hack.

### Priority 4: Security

- [ ] **Sandbox V8** — restrict IPC access to configured pipes only; prevent arbitrary file reads.
- [ ] **Validate ZIP entry paths** in `.cxrl`/`.cxrp` loaders (prevent path traversal).
- [ ] **Replace custom base64 decoder** with `base64` crate.
- [ ] **Named pipe ACLs** — restrict pipe access to expected processes.

### Priority 5: Architecture & Portability

- [ ] **Make CXRD actually binary** — switch from JSON to bincode/postcard for smaller files and faster parsing.
- [ ] **Abstract platform layer** — extract Win32 specifics behind a trait for future Linux/macOS support.
- [ ] **Log rotation** — add file size limit or rotation to the logger.
- [ ] **Add IPC client shutdown mechanism** — currently the poll loop runs forever.
- [ ] **Integration tests** — there are currently only 2 unit tests (easing). No integration tests, no compilation tests, no rendering tests.

---

## Summary

CanvasX is an ambitious custom UI runtime that re-implements significant portions of web rendering (HTML/CSS parsing, flexbox/grid layout, GPU rendering, Canvas 2D, V8 scripting) in Rust. The architecture is sound and the code is generally well-organized. The main gaps are:

1. **Incomplete features** — @keyframes parsing, transitions, flex-wrap, Canvas 2D text, transform animations.
2. **Performance traps** — gradient re-rasterization, O(n) hit testing, single-texture binding.
3. **Security gaps** — unsandboxed V8, no ZIP path validation, no IPC auth.
4. **Dead code & inconsistencies** — unused deps, duplicated flex layout, license mismatch, CXRD "binary" format that's actually JSON.
5. **Windows-only** — deep Win32 dependencies throughout.

For use as a general-purpose UI toolkit, the rendering core and layout engine are the strongest parts. The IPC system and V8 integration add significant capability but also complexity and security surface area.
