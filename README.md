<p align="center">
  <h1 align="center">OpenRender</h1>
  <p align="center">
    <strong>GPU-native 2D scene graph renderer for desktop UIs, widgets, and interactive content.</strong>
  </p>
  <p align="center">
    Written in Rust &nbsp;·&nbsp; Powered by <a href="https://wgpu.rs/">wgpu</a> (Vulkan / DX12) &nbsp;·&nbsp; V8 JavaScript engine
  </p>
  <p align="center">
    <a href="https://github.com/The-Ico2/OpenRender/blob/master/LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue" alt="License"></a>
    <a href="https://github.com/The-Ico2/OpenRender"><img src="https://img.shields.io/badge/status-v0.1.0-orange" alt="Version"></a>
    <a href="https://github.com/The-Ico2/OpenRender">
      <img src="https://img.shields.io/badge/platform-Windows%2010%2F11-brightgreen" alt="Platform">
    </a>
  </p>
</p>

---

## Overview

Prism compiles a practical subset of HTML, CSS, and JavaScript into a binary intermediate format (**PRD**), then renders it directly on the GPU via instanced SDF quads. No browser engine, no WebView — a single Rust library that turns markup into pixels.

```
HTML/CSS/JS  →  Compiler  →  PRD Document  →  Layout  →  Animate  →  Paint  →  GPU
                                                  ↑                              |
                                             IPC (live data)             wgpu (Vulkan/DX12)
```

Prism ships as both a library crate (`prism_runtime`) for embedding into any Rust application and a standalone binary (`prism-rt`) for direct use.

Part of the [OpenDesktop](https://github.com/The-Ico2/OpenDesktop) desktop customization platform.

---

## Key Features

| Feature | Description |
|:--------|:------------|
| **GPU-native rendering** | Every UI element is a single instanced quad. The WGSL fragment shader evaluates SDF rounded rectangles with analytic anti-aliasing — no CPU tessellation, no MSAA. |
| **HTML/CSS compiler** | Parses a subset of HTML and CSS into a fully resolved binary document (PRD). Supports flexbox, grid, variables, gradients, animations, and 50+ CSS properties. |
| **V8 JavaScript engine** | Full ECMAScript support via Google's V8 JIT compiler. DOM querying, class manipulation, Canvas 2D API, timers, and IPC bridge. |
| **Canvas 2D API** | `<canvas>` element with near-complete 2D context — paths, fills, strokes, transforms, gradients, image data, compositing. Software-rendered via tiny-skia, composited on GPU. |
| **Layout engine** | Block flow, Flexbox, CSS Grid, absolute/fixed positioning — all resolved per-frame when dirty. |
| **Animation runtime** | CSS `@keyframes` and `transition` with duration, delay, iteration, direction, fill mode, easing (linear, ease, cubic-bezier). 20+ animatable properties. |
| **Live data binding** | `<data-bind>`, `<data-bar>`, and `<data-bar-stack>` elements display real-time values from IPC (CPU, GPU, RAM, storage, network, etc.). |
| **Text rendering** | GPU text via [glyphon](https://github.com/grovesNL/glyphon) (cosmic-text). Font families, weight, size, line-height, letter-spacing, text-transform, text-align. |
| **SVG support** | `<svg>` elements with `<path>`, `<circle>`, `<rect>`, `<line>`, `<ellipse>`, `<polygon>`, `<polyline>` — auto-rasterized via resvg. |
| **Image support** | PNG, JPEG, WebP, ICO — embedded in the asset bundle. Object-fit modes: fill, contain, cover, none. |
| **IPC bridge** | Named-pipe client for live data streaming. Optional OpenDesktop bridge for 16+ system data sections. JavaScript IPC API for custom commands. |
| **Platform integration** | Windows desktop embedding (WorkerW) for wallpapers, monitor enumeration with DPI awareness. |
| **Editable properties** | Runtime CSS variable overrides driven by `manifest.json` schema + `editable.yaml` user values. |
| **Compile-once caching** | PRD documents cached to disk with SHA-256 hash invalidation. Zero parsing at render time. |

---

## Architecture

### Render Pipeline

Each frame follows the same path:

1. **Layout** — Resolve dimensions and positions for the entire PRD tree (only when dirty)
2. **Animate** — Advance active animations and apply interpolated property values
3. **Script** — Execute queued JavaScript (timers, rAF callbacks, event handlers)
4. **Update data** — Push latest IPC values into data-bound nodes and bars
5. **Prepare text** — Shape and layout all text buffers via glyphon
6. **Paint** — Depth-first tree traversal → flat `Vec<UiInstance>` for the GPU
7. **Render** — Submit instanced draw call + text pass + canvas compositing to wgpu

### Module Map

| Module | Responsibility |
|:-------|:---------------|
| `compiler/` | HTML/CSS → CXRD compilation, asset bundling, editable property bridging |
| `prd/` | Binary scene graph format: nodes, styles, animations, assets, input types |
| `gpu/` | wgpu context, render pipeline, SDF shader, texture manager, instanced rendering |
| `layout/` | Block flow, Flexbox, CSS Grid, absolute/fixed positioning |
| `scene/` | Scene graph coordinator, paint pass, text painter, input handler, app host |
| `animate/` | Animation timeline, keyframe interpolation, easing functions |
| `scripting/` | V8 runtime, DOM bindings, Canvas 2D API, IPC bridge, JS worker |
| `ipc/` | Named-pipe client, protocol types, OpenDesktop bridge (optional) |
| `platform/` | Monitor enumeration, WorkerW desktop embedding (Windows) |

### GPU Rendering

All rendering is instanced — one quad per UI element. The WGSL fragment shader evaluates:

- SDF rounded rectangles with per-corner radius
- Per-side border widths and colors (solid)
- Background colors, gradient textures, and image sampling
- Clip rectangle masking (overflow: hidden/scroll)
- Per-instance opacity
- Analytic anti-aliasing (no MSAA)

Global uniforms provide viewport size, elapsed time, and DPI scale factor.

### Distribution Formats

| Format | Extension | Description |
|:-------|:----------|:------------|
| **PRD** | `.cxrd` | Single compiled document (binary, serde + bincode) |

---

## Web Standard Compatibility

OpenRender implements a practical subset of web standards optimized for high-performance GPU-rendered desktop UIs. The tables below document exact coverage against HTML, CSS, and JavaScript specifications.

> **Legend:**&ensp; ✅ Fully supported &ensp;|&ensp; ⚠️ Partial / parsed but limited &ensp;|&ensp; ❌ Not supported

### HTML Element Support

<details open>
<summary><strong>Containers &amp; Semantic Structure</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<div>` | ✅ | Block container (flex/grid via CSS) |
| `<section>` | ✅ | Semantic block container |
| `<nav>` | ✅ | Navigation container |
| `<header>` | ✅ | Header container |
| `<footer>` | ✅ | Footer container |
| `<main>` | ✅ | Main content container |
| `<aside>` | ✅ | Sidebar container |
| `<article>` | ✅ | Article container |
| `<figure>` | ✅ | Figure container |
| `<figcaption>` | ✅ | Figure caption |

</details>

<details open>
<summary><strong>Text &amp; Inline Content</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<span>` | ✅ | Inline container |
| `<p>` | ✅ | Paragraph — flex row with wrap for inline content |
| `<h1>` – `<h6>` | ✅ | Headings with default sizes (32px – 10.72px), bold |
| `<label>` | ✅ | Inline label |
| `<strong>`, `<b>` | ✅ | Bold text (font-weight: 700) |
| `<em>`, `<i>` | ✅ | Italic text |
| `<small>` | ✅ | Small text |
| `<code>` | ✅ | Inline code |
| `<a>` | ⚠️ | Rendered as inline container; navigation via `data-action="navigate"` |
| `<br>` | ❌ | Not supported — use CSS margin/padding |
| `<hr>` | ❌ | Not supported — use a styled `<div>` |
| `<pre>`, `<blockquote>`, `<abbr>`, `<cite>`, `<q>` | ❌ | Not supported |
| `<mark>`, `<del>`, `<ins>`, `<sub>`, `<sup>`, `<u>` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Lists</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<ul>` | ✅ | Unordered list (rendered as block container) |
| `<ol>` | ✅ | Ordered list (rendered as block container, no auto-numbering) |
| `<li>` | ✅ | List item (rendered as block) |

</details>

<details open>
<summary><strong>Media</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<img>` | ✅ | PNG, JPEG, WebP, ICO from asset bundle. Object-fit: fill, contain, cover, none |
| `<svg>` | ✅ | Auto-rasterized to texture via resvg |
| `<path>`, `<circle>`, `<rect>`, `<line>`, `<ellipse>`, `<polygon>`, `<polyline>` | ✅ | SVG primitives (rasterized, not individually styleable via CSS) |
| `<canvas>` | ✅ | Full Canvas 2D API via tiny-skia backend |
| `<video>` | ❌ | Not supported |
| `<audio>` | ❌ | Not supported |
| `<picture>`, `<source>` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Forms &amp; Input</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<button>` | ✅ | Primary, Secondary, Danger, Ghost, Link variants via `data-variant` |
| `<input type="text">` | ✅ | Text input with placeholder, maxlength, readonly |
| `<input type="password">` | ✅ | Masked input |
| `<input type="number">` | ✅ | Numeric input |
| `<input type="email">` | ✅ | Email input |
| `<input type="search">` | ✅ | Search input |
| `<input type="checkbox">` | ✅ | Checkbox and toggle styles via `data-style` |
| `<input type="range">` | ✅ | Slider with min, max, step |
| `<textarea>` | ✅ | Multi-line text area with rows, maxlength |
| `<select>` | ✅ | Dropdown selector |
| `<option>` | ✅ | Dropdown option items |
| `<form>` | ❌ | No form submission model |
| `<fieldset>`, `<legend>` | ❌ | Not supported |
| `<datalist>`, `<output>`, `<progress>`, `<meter>` | ❌ | Not supported |

</details>

<details open>
<summary><strong>OpenRender Custom Elements</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<data-bind>` | ✅ | Live data display — `binding="cpu.usage"` with format string |
| `<data-bar>` | ✅ | Progress bar bound to IPC data (binding + max-binding + color) |
| `<data-bar-stack>` | ✅ | Multi-segment stacked bar with per-segment bindings |
| `<data-bar-segment>` | ✅ | Individual segment within a stacked bar |
| `<page-content>` | ✅ | Page routing container for multi-page apps |

</details>

<details open>
<summary><strong>Not Supported</strong></summary>

| Element | Status | Notes |
|:--------|:------:|:------|
| `<table>`, `<tr>`, `<td>`, `<th>`, `<thead>`, `<tbody>`, `<tfoot>` | ❌ | No table layout engine — use CSS Grid |
| `<iframe>`, `<embed>`, `<object>` | ❌ | No embedded content model |
| `<dialog>`, `<details>`, `<summary>` | ❌ | Not supported |
| `<template>`, `<slot>` | ❌ | No Web Components model |
| `<script>` (DOM element) | ❌ | Scripts loaded via `src` attribute or inline; not a rendered element |
| `<style>` (DOM element) | ❌ | CSS loaded at compile time; not a rendered element |
| `<link>`, `<meta>`, `<head>`, `<html>`, `<body>` | ❌ | Stripped during parsing |

</details>

---

### CSS Property Support

<details open>
<summary><strong>Layout &amp; Box Model</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `display` | ✅ | `flex`, `grid`, `block`, `inline-block`, `none` |
| `position` | ✅ | `relative`, `absolute`, `fixed` (`static` default, `sticky` not supported) |
| `top`, `right`, `bottom`, `left` | ✅ | For absolute/fixed positioning |
| `z-index` | ✅ | Depth ordering |
| `width`, `height` | ✅ | All units |
| `min-width`, `min-height`, `max-width`, `max-height` | ✅ | All units |
| `margin` (all sides) | ✅ | Shorthand and per-side |
| `padding` (all sides) | ✅ | Shorthand and per-side |
| `box-sizing` | ✅ | `content-box`, `border-box` |
| `overflow`, `overflow-x`, `overflow-y` | ✅ | `visible`, `hidden`, `scroll` (clip rect on GPU) |
| `aspect-ratio` | ✅ | Ratio-based sizing |
| `float` | ⚠️ | Parsed but not implemented in layout |
| `clear` | ⚠️ | Parsed but not implemented |

</details>

<details open>
<summary><strong>Flexbox</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `flex-direction` | ✅ | `row`, `column`, `row-reverse`, `column-reverse` |
| `flex-wrap` | ✅ | `wrap`, `nowrap`, `wrap-reverse` |
| `flex-flow` | ✅ | Shorthand |
| `justify-content` | ✅ | All values |
| `align-items` | ✅ | All values |
| `align-content` | ✅ | All values |
| `align-self` | ✅ | All values |
| `flex-grow` | ✅ | Numeric |
| `flex-shrink` | ✅ | Numeric |
| `flex-basis` | ✅ | All units + `auto` |
| `flex` | ✅ | Shorthand |
| `gap`, `row-gap`, `column-gap` | ✅ | All units |
| `order` | ✅ | Integer ordering |

</details>

<details open>
<summary><strong>CSS Grid</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `grid-template-columns` | ✅ | `px`, `%`, `fr`, `auto`, `min-content`, `max-content`, `repeat()` |
| `grid-template-rows` | ✅ | Same as columns |
| `grid-column`, `grid-row` | ✅ | Span and line-based placement |
| `grid-column-start`, `grid-column-end` | ✅ | Line numbers |
| `grid-row-start`, `grid-row-end` | ✅ | Line numbers |
| `grid-auto-flow` | ✅ | `row`, `column` |
| `grid-gap` | ✅ | Alias for `gap` |
| `grid-template-areas` | ⚠️ | Parsed but limited enforcement |
| `grid-auto-columns`, `grid-auto-rows` | ⚠️ | Parsed but limited |

</details>

<details open>
<summary><strong>Typography</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `font-family` | ✅ | System fonts + bundled fonts |
| `font-size` | ✅ | All units |
| `font-weight` | ✅ | 100–900 and keyword values |
| `font-style` | ✅ | `normal`, `italic` |
| `line-height` | ✅ | Numeric, units, `normal` |
| `letter-spacing` | ✅ | All units |
| `text-align` | ✅ | `left`, `center`, `right`, `justify` |
| `text-transform` | ✅ | `uppercase`, `lowercase`, `capitalize`, `none` |
| `text-decoration` | ✅ | `none`, `underline`, `line-through`, `overline` |
| `text-overflow` | ✅ | `clip`, `ellipsis` |
| `white-space` | ✅ | `normal`, `nowrap`, `pre`, `pre-wrap` |
| `word-break` | ⚠️ | Parsed |
| `word-spacing` | ⚠️ | Parsed |
| `color` | ✅ | All color formats |
| `text-shadow` | ❌ | Not rendered |
| `text-indent` | ❌ | Not rendered |

</details>

<details open>
<summary><strong>Background</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `background-color` | ✅ | Solid colors — rendered in GPU instance |
| `background-image` | ✅ | `linear-gradient()`, `radial-gradient()` — rasterized to texture |
| `background` | ✅ | Shorthand (color and image) |
| `background-size` | ✅ | `auto`, `cover`, `contain`, length values |
| `background-position` | ✅ | Length and keyword values |
| `background-repeat` | ✅ | `repeat`, `no-repeat`, `repeat-x`, `repeat-y` |
| `conic-gradient()` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Border &amp; Outline</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `border-width` | ✅ | Per-side widths — rendered in GPU instance |
| `border-color` | ✅ | Per-side colors — rendered in GPU instance |
| `border-radius` | ✅ | Per-corner radii — SDF-based rendering in shader |
| `border` | ✅ | Shorthand |
| `border-top/right/bottom/left` | ✅ | Per-side shorthand |
| `border-style` | ⚠️ | Parsed but only `solid` is rendered — shader limitation |
| `outline` | ⚠️ | Parsed but not rendered on GPU |
| `border-image` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Visual Effects</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `opacity` | ✅ | Per-instance in GPU — fully rendered |
| `box-shadow` | ✅ | Offset, blur, spread — rendered as layered rects |
| `cursor` | ⚠️ | Parsed and forwarded to OS — not engine-rendered |
| `pointer-events` | ⚠️ | Affects hit testing only |
| `filter` | ⚠️ | Parsed but not rendered on GPU |
| `backdrop-filter` | ⚠️ | Blur approximated via alpha/luminance adjustment — not true blur |
| `mix-blend-mode` | ⚠️ | Parsed but not rendered |
| `visibility` | ⚠️ | Parsed |
| `clip-path` | ❌ | Not supported |
| `mask` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Transform</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `transform` | ⚠️ | Parsed to style struct but not applied to GPU instances — no visual effect |
| `transform-origin` | ⚠️ | Parsed only |
| `rotate`, `scale`, `translate` | ⚠️ | Individual properties parsed but not rendered |
| `perspective` | ❌ | Not supported (no 3D pipeline) |
| `transform-style: preserve-3d` | ❌ | Not supported |

</details>

<details open>
<summary><strong>Animation &amp; Transition</strong></summary>

| Property | Status | Notes |
|:---------|:------:|:------|
| `animation` | ✅ | Shorthand |
| `animation-name` | ✅ | References `@keyframes` |
| `animation-duration` | ✅ | Time values |
| `animation-delay` | ✅ | Time values |
| `animation-iteration-count` | ✅ | Number or `infinite` |
| `animation-direction` | ✅ | `normal`, `reverse`, `alternate`, `alternate-reverse` |
| `animation-fill-mode` | ✅ | `none`, `forwards`, `backwards`, `both` |
| `animation-timing-function` | ✅ | `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out`, `cubic-bezier()` |
| `transition` | ✅ | Shorthand |
| `transition-property` | ✅ | Target properties |
| `transition-duration` | ✅ | Time values |
| `transition-delay` | ✅ | Time values |
| `transition-timing-function` | ✅ | Same as animation |

**Animatable properties:** `opacity`, `background-color`, `color`, `border-color`, `border-radius`, `border-width`, `width`, `height`, `min-width`, `min-height`, `max-width`, `max-height`, `padding` (all sides), `margin` (all sides), `gap`, `font-size`

</details>

<details open>
<summary><strong>Selectors</strong></summary>

| Selector | Syntax | Status | Notes |
|:---------|:-------|:------:|:------|
| Universal | `*` | ✅ | Matches all elements |
| Type | `div`, `p` | ✅ | Tag name matching |
| Class | `.name` | ✅ | Class matching |
| ID | `#name` | ✅ | ID matching |
| Descendant | `a b` | ✅ | Ancestor-descendant |
| Child | `a > b` | ⚠️ | Parsed but treated as descendant |
| Compound | `div.class#id` | ✅ | Multiple simple selectors |
| `:hover` | `:hover` | ✅ | Mouse over state |
| `:active` | `:active` | ✅ | Mouse down state |
| `:focus` | `:focus` | ✅ | Focus state |
| `:focus-visible` | `:focus-visible` | ✅ | Keyboard focus |
| `:checked` | `:checked` | ✅ | Checkbox/toggle state |
| `:disabled` | `:disabled` | ✅ | Disabled form state |
| `:first-child`, `:last-child` | | ⚠️ | Parsed; limited runtime |
| `:nth-child(n)` | | ⚠️ | Parsed; limited runtime |
| `:not()`, `:is()`, `:has()`, `:where()` | | ⚠️ | Parsed; limited enforcement |
| `::before`, `::after` | | ⚠️ | Parsed; limited support |
| Adjacent/general sibling | `+`, `~` | ❌ | Not supported |
| Attribute selectors | `[attr=value]` | ⚠️ | Parsed; limited matching |

</details>

<details open>
<summary><strong>Units</strong></summary>

| Unit | Status | Notes |
|:-----|:------:|:------|
| `px` | ✅ | Absolute pixels |
| `%` | ✅ | Percentage of parent |
| `em` | ✅ | Relative to element font-size |
| `rem` | ✅ | Relative to root font-size (16px) |
| `vw`, `vh` | ⚠️ | Approximated as percentage |
| `vmin`, `vmax` | ⚠️ | Approximated as percentage |
| `auto` | ✅ | Intrinsic sizing |
| `fr` | ✅ | Grid fractional unit |
| `calc()` | ⚠️ | First numeric term extracted only |
| `min()`, `max()`, `clamp()` | ❌ | Not supported |
| `ch`, `ex` | ❌ | Not supported |
| `cm`, `mm`, `in`, `pt`, `pc` | ❌ | Print units not supported |

</details>

<details open>
<summary><strong>At-Rules</strong></summary>

| Rule | Status | Notes |
|:-----|:------:|:------|
| `@keyframes` | ✅ | Full animation keyframe support |
| `@media` | ⚠️ | Parsed; limited runtime query matching |
| `@import` | ⚠️ | Recognized for URL-based stylesheet imports |
| `@font-face` | ⚠️ | Recognized; font loading limited |
| `@supports` | ⚠️ | Parsed |
| `@layer` | ⚠️ | Recognized |
| `@container` | ⚠️ | Recognized; not enforced |

</details>

<details open>
<summary><strong>Functions &amp; Colors</strong></summary>

| Function | Status | Notes |
|:---------|:------:|:------|
| `rgb()`, `rgba()` | ✅ | Full support |
| `hsl()`, `hsla()` | ✅ | Full support |
| `#RGB`, `#RRGGBB`, `#RRGGBBAA` | ✅ | Hex colors |
| Named colors (148) | ✅ | CSS Level 4 named colors |
| `var(--name)` | ✅ | CSS custom properties with fallback |
| `linear-gradient()` | ✅ | Rasterized to texture |
| `radial-gradient()` | ✅ | Rasterized to texture |
| `url()` | ✅ | Background images from asset bundle |
| `cubic-bezier()` | ✅ | Animation timing function |
| `conic-gradient()` | ❌ | Not supported |
| `calc()` | ⚠️ | Extracts first numeric term only |
| `min()`, `max()`, `clamp()` | ❌ | Not supported |

</details>

---

### JavaScript Support

OpenRender includes a full V8 JavaScript engine. Scripts can be embedded inline or loaded via `src` attribute.

<details open>
<summary><strong>DOM API</strong></summary>

| API | Status | Notes |
|:----|:------:|:------|
| `document.getElementById(id)` | ✅ | Node lookup by ID |
| `document.querySelector(sel)` | ✅ | CSS selector query |
| `document.querySelectorAll(sel)` | ✅ | Multiple element query |
| `element.classList.add()` | ✅ | Add CSS class |
| `element.classList.remove()` | ✅ | Remove CSS class |
| `element.classList.toggle()` | ✅ | Toggle CSS class |
| `element.style.*` | ✅ | Inline style manipulation |
| `element.textContent` | ✅ | Get/set text content |
| `element.innerHTML` | ⚠️ | Parse and inject HTML subtree |
| `element.addEventListener()` | ⚠️ | Limited event types |
| `element.removeEventListener()` | ⚠️ | Event removal |
| Full DOM tree manipulation | ❌ | No `createElement`, `appendChild`, `removeChild` |

</details>

<details open>
<summary><strong>Canvas 2D API</strong></summary>

| Category | Methods | Status |
|:---------|:--------|:------:|
| **Drawing** | `fillRect()`, `strokeRect()`, `clearRect()` | ✅ |
| **Paths** | `beginPath()`, `closePath()`, `moveTo()`, `lineTo()`, `arc()`, `bezierCurveTo()`, `quadraticCurveTo()`, `fill()`, `stroke()` | ✅ |
| **Transform** | `save()`, `restore()`, `translate()`, `rotate()`, `scale()`, `transform()` | ✅ |
| **Style** | `fillStyle`, `strokeStyle`, `lineWidth`, `globalAlpha`, `globalCompositeOperation` | ✅ |
| **Gradient** | `createLinearGradient()`, `createRadialGradient()`, `addColorStop()` | ✅ |
| **Image data** | `getImageData()`, `putImageData()` | ✅ |
| **Clipping** | `clip()` | ✅ |
| **Text** | `font`, `fillText()`, `strokeText()` | ⚠️ |
| **Image draw** | `drawImage()` | ⚠️ |

</details>

<details open>
<summary><strong>Globals &amp; Timers</strong></summary>

| API | Status | Notes |
|:----|:------:|:------|
| `console.log()`, `.warn()`, `.error()` | ✅ | Output to host logger |
| `setTimeout()`, `setInterval()` | ✅ | Timer scheduling |
| `requestAnimationFrame()` | ✅ | Per-frame callback |
| `Math`, `JSON`, `String`, `Array`, `Object`, `Date` | ✅ | Standard JS builtins |
| `__or_sendIpc(ns, cmd, args)` | ✅ | Send IPC commands to host |
| `fetch()`, `XMLHttpRequest` | ❌ | Not available — use IPC bridge |
| `Worker`, `SharedWorker` | ❌ | Not available |
| `import`, `require` | ❌ | No module system |
| `eval()` | ❌ | Disabled for security |

</details>

---

## Usage

### As a Library

```toml
[dependencies]
prism-runtime = { path = "../OpenRender" }
```

```rust
use openrender_runtime::{GpuContext, SceneGraph};
use openrender_runtime::compiler::html::compile_html;
use openrender_runtime::gpu::renderer::Renderer;
use openrender_runtime::cxrd::document::SceneType;

// Compile HTML/CSS to PRD
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
prism-rt --wallpaper --source index.html --css style.css --monitor 0
prism-rt --widget --source panel.html --fps 60
prism-rt --config --source settings.html
```

| Flag | Description |
|:-----|:------------|
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

OpenRender nodes can bind to live data via the IPC bridge. Data keys use dot-notation paths.

```html
<data-bind binding="cpu.usage" format="{value}%"></data-bind>

<data-bar binding="ram.used_bytes" max-binding="ram.total_bytes" style="background: #4a9eff"></data-bar>

<data-bar-stack max-binding="storage.total_bytes">
    <data-bar-segment binding="storage.disks.0.used_bytes" style="background: var(--disk0)"></data-bar-segment>
    <data-bar-segment binding="storage.disks.1.used_bytes" style="background: var(--disk1)"></data-bar-segment>
</data-bar-stack>
```

JavaScript can also send IPC commands directly:

```js
__or_sendIpc("sysdata", "get_cpu", null);
```

When connected to OpenDesktop, the bridge polls 16+ data sections (time, CPU, GPU, RAM, storage, displays, network, Wi-Fi, Bluetooth, audio, keyboard, mouse, power, idle, system, processes) and flattens them into a key-value map consumed by the scene graph.

---

## Dependencies

| Category | Crate | Version |
|:---------|:------|:--------|
| GPU | `wgpu` | 28.0 |
| Windowing | `winit` | 0.30 |
| Text rendering | `glyphon` | 0.10 |
| GPU buffers | `bytemuck` | 1.25 |
| Images | `image` | 0.25 |
| JavaScript | `v8` | 146.3 |
| Canvas 2D | `tiny-skia` | 0.12 |
| SVG | `resvg` | 0.47 |
| Serialization | `serde`, `serde_json`, `bincode` | 1.0 / 1.0 / 2.0 |
| CSS parsing | `cssparser` | 0.36 |
| Platform | `windows` | 0.62 |
| File watching | `notify` | 8.2 |
| Archives | `zip` | 8.1 |
| Hashing | `sha2`, `hex` | 0.10 / 0.4 |
| Concurrency | `parking_lot` | 0.12 |

---

## Requirements

- **OS:** Windows 10 or 11
- **GPU:** Vulkan or DirectX 12 capable
- **Runtime:** OpenDesktop Backend — only required for live data binding

---

## Project Status

Under active development (`v0.1.0`). APIs, document format, and behavior may change.

---

## License

Apache 2.0 — see [LICENSE](LICENSE).

---

## Contact

- **Discord:** the_ico2
- **X:** [@The_Ico2](https://x.com/The_Ico2)
