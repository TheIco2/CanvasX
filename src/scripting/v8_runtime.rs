// prism-runtime/src/scripting/v8_runtime.rs
//
// JavaScript runtime powered by V8 (via the `v8` crate / rusty_v8).
// Drop-in replacement for the boa_engine-based runtime — same public API,
// but backed by Google's V8 JIT engine for 100–1000× faster JS execution.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use crate::prd::document::PrdDocument;
use crate::prd::node::{NodeKind, NodeId};
use crate::compiler::css::{CssRule, apply_property};
use crate::compiler::html::{compound_selector_matches, AncestorInfo};
use crate::layout::engine::compute_layout;
use crate::ipc::client::send_ipc_request_to;
use crate::ipc::protocol::IpcRequest;
use crate::scripting::canvas2d::{CanvasManager, CanvasId, GradientDef, parse_css_color};

// ═══════════════════════════════════════════════════════════════════════════
// V8 platform initialisation (once per process)
// ═══════════════════════════════════════════════════════════════════════════

static V8_INIT: std::sync::Once = std::sync::Once::new();

fn ensure_v8_initialized() {
    V8_INIT.call_once(|| {
        let platform = v8::new_default_platform(0, false).make_shared();
        v8::V8::initialize_platform(platform);
        v8::V8::initialize();
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Thread-local shared state (identical to boa runtime)
// ═══════════════════════════════════════════════════════════════════════════

thread_local! {
    static RUNTIME_STATE: RefCell<Option<StateRef>> = RefCell::new(None);
    /// Buffer for console messages emitted by JS code (level, message).
    static CONSOLE_BUFFER: RefCell<Vec<(i32, String)>> = RefCell::new(Vec::new());
}

fn with_state<F, R>(f: F) -> R
where
    F: FnOnce(&mut SharedState) -> R,
{
    RUNTIME_STATE.with(|cell| {
        let opt = cell.borrow();
        let state_ref = opt.as_ref().expect("RUNTIME_STATE not set");
        let mut state = state_ref.borrow_mut();
        f(&mut state)
    })
}

fn reachable_nodes(doc: &PrdDocument) -> Vec<bool> {
    let mut reachable = vec![false; doc.nodes.len()];
    let mut stack = vec![doc.root];
    while let Some(id) = stack.pop() {
        let idx = id as usize;
        if idx >= doc.nodes.len() || reachable[idx] { continue; }
        reachable[idx] = true;
        for &child in &doc.nodes[idx].children {
            stack.push(child);
        }
    }
    reachable
}

fn get_reachable(st: &mut SharedState) -> Vec<bool> {
    if let Some(ref cached) = st.reachable_cache {
        return cached.clone();
    }
    let reachable = reachable_nodes(&st.document);
    st.reachable_cache = Some(reachable.clone());
    reachable
}

/// Decode HTML character entities in a text string.
/// Handles named entities (&amp;, &lt;, &gt;, &quot;, &apos;, &nbsp;, arrows, etc.)
/// and numeric entities (&#NNN; and &#xHHH;).
fn decode_html_entities(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '&' {
            let mut entity = String::new();
            let mut found_semi = false;
            for _ in 0..10 {
                match chars.peek() {
                    Some(&';') => { chars.next(); found_semi = true; break; }
                    Some(&ch) if ch.is_alphanumeric() || ch == '#' => { entity.push(ch); chars.next(); }
                    _ => break,
                }
            }
            if found_semi {
                match entity.as_str() {
                    "amp" => result.push('&'),
                    "lt" => result.push('<'),
                    "gt" => result.push('>'),
                    "quot" => result.push('"'),
                    "apos" => result.push('\''),
                    "nbsp" => result.push('\u{00A0}'),
                    "rsaquo" => result.push('\u{203A}'),
                    "lsaquo" => result.push('\u{2039}'),
                    "larr" => result.push('\u{2190}'),
                    "rarr" => result.push('\u{2192}'),
                    "uarr" => result.push('\u{2191}'),
                    "darr" => result.push('\u{2193}'),
                    "mdash" => result.push('\u{2014}'),
                    "ndash" => result.push('\u{2013}'),
                    "bull" => result.push('\u{2022}'),
                    "hellip" => result.push('\u{2026}'),
                    "copy" => result.push('\u{00A9}'),
                    "reg" => result.push('\u{00AE}'),
                    "trade" => result.push('\u{2122}'),
                    "times" => result.push('\u{00D7}'),
                    "divide" => result.push('\u{00F7}'),
                    "laquo" => result.push('\u{00AB}'),
                    "raquo" => result.push('\u{00BB}'),
                    other => {
                        // Numeric entity: &#NNN; or &#xHHH;
                        if let Some(stripped) = other.strip_prefix('#') {
                            let code = if let Some(hex) = stripped.strip_prefix('x') {
                                u32::from_str_radix(hex, 16).ok()
                            } else {
                                stripped.parse::<u32>().ok()
                            };
                            if let Some(cp) = code {
                                if let Some(ch) = char::from_u32(cp) {
                                    result.push(ch);
                                } else {
                                    result.push_str(&format!("&{};", other));
                                }
                            } else {
                                result.push_str(&format!("&{};", other));
                            }
                        } else {
                            // Unknown named entity — pass through as-is
                            result.push_str(&format!("&{};", other));
                        }
                    }
                }
            } else {
                // No semicolon found — not a valid entity, emit as-is
                result.push('&');
                result.push_str(&entity);
            }
        } else {
            result.push(c);
        }
    }
    result
}

/// Shared mutable state accessible from both Rust and JS native functions.
pub struct SharedState {
    pub document: PrdDocument,
    pub css_rules: Vec<CssRule>,
    pub css_variables: HashMap<String, String>,
    pub canvas_manager: CanvasManager,
    pub node_canvas_map: HashMap<NodeId, CanvasId>,
    pub layout_dirty: bool,
    pub raf_pending: bool,
    pub data_values: HashMap<String, String>,
    pub reachable_cache: Option<Vec<bool>>,
    pub canvas_node_map: HashMap<CanvasId, NodeId>,
    pub viewport_width: u32,
    pub viewport_height: u32,
    /// Scripts deferred from innerHTML `<script>` blocks — executed after DOM update.
    pub deferred_scripts: Vec<String>,
    /// Set when new image assets are added dynamically (e.g. SVG rasterization
    /// via innerHTML) and need to be uploaded to the GPU.
    pub assets_dirty: bool,
}

pub type StateRef = Rc<RefCell<SharedState>>;

// ═══════════════════════════════════════════════════════════════════════════
// Utility helpers
// ═══════════════════════════════════════════════════════════════════════════

/// Convert a camelCase JS property name to CSS kebab-case.
/// e.g. "backgroundColor" → "background-color", "background" → "background"
fn camel_to_kebab(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 4);
    for ch in s.chars() {
        if ch.is_uppercase() {
            out.push('-');
            out.push(ch.to_lowercase().next().unwrap());
        } else {
            out.push(ch);
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════════════════
// V8 argument extraction helpers
// ═══════════════════════════════════════════════════════════════════════════

#[inline]
fn v8_str(scope: &mut v8::PinScope<'_, '_>, args: &v8::FunctionCallbackArguments, i: i32) -> String {
    args.get(i).to_rust_string_lossy(scope)
}

#[inline]
fn v8_i32(args: &v8::FunctionCallbackArguments, scope: &mut v8::PinScope<'_, '_>, i: i32) -> i32 {
    args.get(i).int32_value(scope).unwrap_or(0)
}

#[inline]
#[allow(dead_code)]
fn v8_f64(args: &v8::FunctionCallbackArguments, scope: &mut v8::PinScope<'_, '_>, i: i32) -> f64 {
    args.get(i).number_value(scope).unwrap_or(0.0)
}

#[inline]
fn v8_f32(args: &v8::FunctionCallbackArguments, scope: &mut v8::PinScope<'_, '_>, i: i32) -> f32 {
    args.get(i).number_value(scope).unwrap_or(0.0) as f32
}

// ═══════════════════════════════════════════════════════════════════════════
// JsRuntime — public API (matches the boa-based runtime exactly)
// ═══════════════════════════════════════════════════════════════════════════

pub struct JsRuntime {
    isolate: v8::OwnedIsolate,
    context: v8::Global<v8::Context>,
    pub state: StateRef,
    #[allow(dead_code)]
    initialized: bool,
    epoch: Instant,
    epoch_offset_ms: f64,
    raf_tick_fn: Option<v8::Global<v8::Function>>,
}

impl JsRuntime {
    /// Create a new V8-backed JS runtime and register all native bindings.
    pub fn new(
        document: PrdDocument,
        css_rules: Vec<CssRule>,
        css_variables: HashMap<String, String>,
    ) -> Self {
        ensure_v8_initialized();

        let state = Rc::new(RefCell::new(SharedState {
            document,
            css_rules,
            css_variables,
            canvas_manager: CanvasManager::new(),
            node_canvas_map: HashMap::new(),
            layout_dirty: false,
            raf_pending: false,
            data_values: HashMap::new(),
            reachable_cache: None,
            canvas_node_map: HashMap::new(),
            viewport_width: 1920,
            viewport_height: 1080,
            deferred_scripts: Vec::new(),
            assets_dirty: false,
        }));

        RUNTIME_STATE.with(|cell| {
            *cell.borrow_mut() = Some(state.clone());
        });

        let mut isolate = v8::Isolate::new(v8::CreateParams::default());

        // Create context and register all native functions on the global object.
        let context = {
            let hs = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let mut hs = hs.init();
            let ctx = v8::Context::new(&hs, Default::default());
            let cs = &mut v8::ContextScope::new(&mut hs, ctx);
            register_all_functions(cs);
            v8::Global::new(cs, ctx)
        };

        // Inject the JavaScript DOM/Canvas shim.
        {
            let hs = std::pin::pin!(v8::HandleScope::new(&mut isolate));
            let mut hs = hs.init();
            let ctx = v8::Local::new(&hs, &context);
            let cs = &mut v8::ContextScope::new(&mut hs, ctx);

            let code = v8::String::new(cs, JS_SHIM).unwrap();
            let tc = std::pin::pin!(v8::TryCatch::new(cs));
            let tc = tc.init();
            match v8::Script::compile(&tc, code, None) {
                Some(script) => match script.run(&tc) {
                    Some(_) => log::info!("[PRISM][JS] V8 shim injected OK ({} bytes)", JS_SHIM.len()),
                    None => {
                        let msg = tc.exception()
                            .map(|e| e.to_rust_string_lossy(&tc))
                            .unwrap_or_default();
                        log::error!("[PRISM][JS] V8 shim execution FAILED: {}", msg);
                    }
                },
                None => {
                    let msg = tc.exception()
                        .map(|e| e.to_rust_string_lossy(&tc))
                        .unwrap_or_default();
                    log::error!("[PRISM][JS] V8 shim compilation FAILED: {}", msg);
                }
            }
        }

        Self {
            isolate,
            context,
            state,
            initialized: false,
            epoch: Instant::now(),
            epoch_offset_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64() * 1000.0,
            raf_tick_fn: None,
        }
    }

    /// Initialize canvas elements — create CanvasBuffer for each <canvas> node.
    pub fn init_canvases(&mut self, viewport_width: u32, viewport_height: u32) {
        let mut state = self.state.borrow_mut();
        state.viewport_width = viewport_width.max(1);
        state.viewport_height = viewport_height.max(1);
        let count = state.document.nodes.len();
        for i in 0..count {
            if state.document.nodes[i].tag.as_deref() == Some("canvas") {
                let (mut w, mut h) = match state.document.nodes[i].kind {
                    NodeKind::Canvas { width, height } => (width.max(1), height.max(1)),
                    _ => (viewport_width.max(1), viewport_height.max(1)),
                };
                if w == 0 { w = viewport_width.max(1); }
                if h == 0 { h = viewport_height.max(1); }
                let node_id = state.document.nodes[i].id;
                let canvas_id = state.canvas_manager.create_canvas(w, h);
                state.node_canvas_map.insert(node_id, canvas_id);
                state.canvas_node_map.insert(canvas_id, node_id);
                log::info!("Created canvas buffer {} for node {} ({}×{})", canvas_id, node_id, w, h);
            }
        }
    }

    /// Set this runtime's state in the thread-local.
    pub fn activate(&self) {
        RUNTIME_STATE.with(|cell| {
            *cell.borrow_mut() = Some(self.state.clone());
        });
    }

    /// Execute script source code.
    pub fn execute(&mut self, source: &str, name: &str) {
        self.activate();
        log::warn!("[PRISM][JS] Executing script '{}' ({} bytes)", name, source.len());

        let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
        let mut hs = hs.init();
        let ctx = v8::Local::new(&hs, &self.context);
        let cs = &mut v8::ContextScope::new(&mut hs, ctx);

        let code = match v8::String::new(cs, source) {
            Some(s) => s,
            None => {
                log::error!("[PRISM][JS] Failed to create V8 string for '{}'", name);
                return;
            }
        };

        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let tc = tc.init();
        match v8::Script::compile(&tc, code, None) {
            Some(script) => match script.run(&tc) {
                Some(_) => {
                    log::debug!("[PRISM][JS] Script '{}' completed OK", name);
                    let state = self.state.borrow();
                    log::warn!(
                        "[PRISM][JS] Doc state: {} nodes, {} canvases, layout_dirty={}",
                        state.document.nodes.len(),
                        state.node_canvas_map.len(),
                        state.layout_dirty,
                    );
                }
                None => {
                    let msg = tc.exception()
                        .map(|e| e.to_rust_string_lossy(&tc))
                        .unwrap_or_default();
                    log::error!("[PRISM][JS] Script '{}' THREW: {}", name, msg);
                }
            },
            None => {
                let msg = tc.exception()
                    .map(|e| e.to_rust_string_lossy(&tc))
                    .unwrap_or_default();
                log::error!("[PRISM][JS] Script '{}' compile error: {}", name, msg);
            }
        }
    }

    /// Execute a script file from disk.
    pub fn execute_file(&mut self, path: &Path) {
        self.activate();
        log::warn!("[PRISM][JS] Loading script file: {}", path.display());
        match std::fs::read_to_string(path) {
            Ok(source) => {
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                self.execute(&source, name);
            }
            Err(e) => log::error!("[PRISM][JS] Failed to read script '{}': {}", path.display(), e),
        }
    }

    /// Resolve and cache the global __or_raf_tick function.
    pub fn cache_raf_tick_fn(&mut self) {
        let global_fn = {
            let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
            let mut hs = hs.init();
            let ctx = v8::Local::new(&hs, &self.context);
            let cs = &mut v8::ContextScope::new(&mut hs, ctx);

            let global = ctx.global(cs);
            let key = v8::String::new(cs, "__or_raf_tick").unwrap();
            global.get(cs, key.into()).and_then(|val| {
                if val.is_function() {
                    let func: v8::Local<v8::Function> = { val.cast() };
                    Some(v8::Global::new(cs, func))
                } else {
                    None
                }
            })
        };

        if let Some(func) = global_fn {
            self.raf_tick_fn = Some(func);
            log::info!("[PRISM][JS] Cached __or_raf_tick function for direct calls");
        } else {
            log::warn!("[PRISM][JS] __or_raf_tick not found or not callable — will fall back to eval");
        }
    }

    /// Run one frame tick. Returns true if any canvas was modified.
    pub fn tick(&mut self, _dt: f32) -> bool {
        self.activate();
        let now_ms = self.epoch.elapsed().as_secs_f64() * 1000.0 + self.epoch_offset_ms;
        let tick_fn_global = self.raf_tick_fn.clone();

        {
            let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
            let mut hs = hs.init();
            let ctx = v8::Local::new(&hs, &self.context);
            let cs = &mut v8::ContextScope::new(&mut hs, ctx);

            if let Some(ref gfn) = tick_fn_global {
                let func = v8::Local::new(cs, gfn);
                let recv = v8::undefined(cs).into();
                let ts = v8::Number::new(cs, now_ms);
                let args = [ts.into()];
                let tc = std::pin::pin!(v8::TryCatch::new(cs));
                let tc = tc.init();
                if func.call(&tc, recv, &args).is_none() {
                    if let Some(ex) = tc.exception() {
                        let msg = ex.to_rust_string_lossy(&tc);
                        log::error!("[PRISM][JS] tick error: {}", msg);
                    }
                }
            } else {
                let code_str = format!(
                    "if(typeof __or_raf_tick==='function')__or_raf_tick({});",
                    now_ms
                );
                if let Some(code) = v8::String::new(cs, &code_str) {
                    let tc = std::pin::pin!(v8::TryCatch::new(cs));
                    let tc = tc.init();
                    if let Some(script) = v8::Script::compile(&tc, code, None) {
                        if script.run(&tc).is_none() {
                            if let Some(ex) = tc.exception() {
                                let msg = ex.to_rust_string_lossy(&tc);
                                log::error!("[PRISM][JS] tick eval error: {}", msg);
                            }
                        }
                    }
                }
            }
        }

        let state = self.state.borrow();
        state.canvas_manager.buffers.values().any(|c| c.dirty)
    }

    /// Mark all canvas buffers as clean (after GPU upload).
    pub fn clear_dirty_flags(&mut self) {
        let mut state = self.state.borrow_mut();
        for canvas in state.canvas_manager.buffers.values_mut() {
            canvas.dirty = false;
        }
        state.canvas_manager.dirty_count = 0;
    }

    /// Collect stale gradients.
    pub fn gc_gradients(&mut self) {
        self.state.borrow_mut().canvas_manager.gc_gradients();
    }

    /// Get all dirty canvas buffers for GPU texture upload.
    pub fn dirty_canvases(&self) -> Vec<(CanvasId, Option<NodeId>, u32, u32, Vec<u8>)> {
        let state = self.state.borrow();
        let mut result = Vec::new();
        for (&cid, canvas) in &state.canvas_manager.buffers {
            if canvas.dirty {
                let node_id = state.canvas_node_map.get(&cid).copied();
                result.push((cid, node_id, canvas.width, canvas.height, canvas.pixels().to_vec()));
            }
        }
        result
    }

    /// Check and clear the layout_dirty flag.
    pub fn take_layout_dirty(&mut self) -> bool {
        let mut state = self.state.borrow_mut();
        let dirty = state.layout_dirty;
        state.layout_dirty = false;
        dirty
    }

    /// Check and clear the assets_dirty flag (set when innerHTML adds new
    /// image textures that need uploading to the GPU).
    pub fn take_assets_dirty(&mut self) -> bool {
        let mut state = self.state.borrow_mut();
        let dirty = state.assets_dirty;
        state.assets_dirty = false;
        dirty
    }

    /// Get the document (for layout/paint passes).
    pub fn document(&self) -> std::cell::Ref<'_, PrdDocument> {
        std::cell::Ref::map(self.state.borrow(), |s| &s.document)
    }

    /// Replace the JS runtime's document with an updated copy.
    /// Used after content swaps so that JS-driven DOM syncs don't overwrite
    /// the newly swapped content with a stale snapshot.
    pub fn sync_document(&self, doc: &PrdDocument) {
        self.state.borrow_mut().document = doc.clone();
    }

    /// Get a mutable reference to the shared state.
    pub fn state_mut(&self) -> std::cell::RefMut<'_, SharedState> {
        self.state.borrow_mut()
    }

    /// Re-apply CSS rules to the entire document tree.
    pub fn restyle(&self) {
        self.activate();
        let mut state = self.state.borrow_mut();
        let rules = state.css_rules.clone();
        let vars = state.css_variables.clone();
        crate::compiler::html::restyle_document(&mut state.document, &rules, &vars);
        let vw = state.viewport_width as f32;
        let vh = state.viewport_height as f32;
        compute_layout(&mut state.document, vw, vh);
        log::debug!("[PRISM][JS] Restyled document with {} rules", rules.len());
    }

    /// Drain all console messages buffered since the last call.
    /// Returns a `Vec` of `(level, message)` tuples where level is:
    /// 0 = debug, 1 = info/log, 2 = warn, 3 = error.
    pub fn drain_console(&self) -> Vec<(i32, String)> {
        CONSOLE_BUFFER.with(|buf| std::mem::take(&mut *buf.borrow_mut()))
    }

    /// Dispatch a DOM event to JS element listeners (with bubbling).
    /// Calls the JS-side `__or_dispatchDomEvent(nodeId, type)` function.
    pub fn dispatch_dom_event(&mut self, node_id: u32, event_type: &str) {
        self.activate();
        let code = format!(
            "if(typeof __or_dispatchDomEvent==='function')__or_dispatchDomEvent({},\"{}\");",
            node_id, event_type
        );
        let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
        let mut hs = hs.init();
        let ctx = v8::Local::new(&hs, &self.context);
        let cs = &mut v8::ContextScope::new(&mut hs, ctx);
        let source = v8::String::new(cs, &code).unwrap();
        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let tc = tc.init();
        if let Some(script) = v8::Script::compile(&tc, source, None) {
            if script.run(&tc).is_none() {
                if let Some(exc) = tc.exception() {
                    log::error!("[PRISM][JS] Event dispatch error: {}", exc.to_rust_string_lossy(&tc));
                }
            }
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Native function registration
// ═══════════════════════════════════════════════════════════════════════════

fn register_all_functions(scope: &mut v8::PinScope<'_, '_>) {
    let global = scope.get_current_context().global(scope);

    macro_rules! set_fn {
        ($name:expr, $cb:ident) => {{
            let key = v8::String::new(scope, $name).unwrap();
            let func = v8::Function::new(scope, $cb).unwrap();
            global.set(scope, key.into(), func.into());
        }};
    }

    // Console
    set_fn!("__or_log", cx_log);
    // Performance
    set_fn!("__or_performance_now", cx_performance_now);
    // DOM
    set_fn!("__or_getElementById", cx_get_element_by_id);
    set_fn!("__or_querySelector", cx_query_selector);
    set_fn!("__or_querySelectorAll", cx_query_selector_all);
    set_fn!("__or_createElement", cx_create_element);
    set_fn!("__or_getTextContent", cx_get_text_content);
    set_fn!("__or_setTextContent", cx_set_text_content);
    set_fn!("__or_setInnerHTML", cx_set_inner_html);
    set_fn!("__or_getNodeAttribute", cx_get_node_attribute);
    set_fn!("__or_setNodeAttribute", cx_set_node_attribute);
    set_fn!("__or_classListOp", cx_class_list_op);
    set_fn!("__or_getStyle", cx_get_style);
    set_fn!("__or_setStyle", cx_set_style);
    set_fn!("__or_getComputedStyleVar", cx_get_computed_style_var);
    set_fn!("__or_setRootStyleProperty", cx_set_root_style_property);
    set_fn!("__or_getNodeTag", cx_get_node_tag);
    set_fn!("__or_getNodeChildren", cx_get_node_children);
    set_fn!("__or_getNodeId", cx_get_node_id);
    set_fn!("__or_appendChild", cx_append_child);
    set_fn!("__or_removeChild", cx_remove_child);
    set_fn!("__or_getParentNode", cx_get_parent_node);
    set_fn!("__or_insertBefore", cx_insert_before);
    set_fn!("__or_getNodeClientSize", cx_get_node_client_size);
    set_fn!("__or_getNodeRect", cx_get_node_rect);
    // Canvas 2D
    set_fn!("__or_getCanvasId", cx_get_canvas_id);
    set_fn!("__or_canvasSetSize", cx_canvas_set_size);
    set_fn!("__or_canvasGetSize", cx_canvas_get_size);
    set_fn!("__or_c2d", cx_c2d);
    set_fn!("__or_c2d_setFillStyle", cx_c2d_set_fill_style);
    set_fn!("__or_c2d_setStrokeStyle", cx_c2d_set_stroke_style);
    set_fn!("__or_c2d_setFillGradient", cx_c2d_set_fill_gradient);
    set_fn!("__or_c2d_setStrokeGradient", cx_c2d_set_stroke_gradient);
    set_fn!("__or_c2d_setFillPattern", cx_c2d_set_fill_pattern);
    set_fn!("__or_c2d_setBlendMode", cx_c2d_set_blend_mode);
    set_fn!("__or_c2d_setFont", cx_c2d_set_font);
    set_fn!("__or_c2d_setTextAlign", cx_c2d_set_text_align);
    set_fn!("__or_c2d_setTextBaseline", cx_c2d_set_text_baseline);
    set_fn!("__or_c2d_setLineCap", cx_c2d_set_line_cap);
    set_fn!("__or_c2d_setLineJoin", cx_c2d_set_line_join);
    set_fn!("__or_c2d_setMiterLimit", cx_c2d_set_miter_limit);
    set_fn!("__or_c2d_fillText", cx_c2d_fill_text);
    set_fn!("__or_c2d_drawImage", cx_c2d_draw_image);
    set_fn!("__or_c2d_createRadialGradient", cx_c2d_create_radial_gradient);
    set_fn!("__or_c2d_createLinearGradient", cx_c2d_create_linear_gradient);
    set_fn!("__or_c2d_gradientAddStop", cx_c2d_gradient_add_stop);
    set_fn!("__or_c2d_createPattern", cx_c2d_create_pattern);
    set_fn!("__or_c2d_fillRectGrad", cx_c2d_fill_rect_grad);
    set_fn!("__or_c2d_fillRectPattern", cx_c2d_fill_rect_pattern);
    set_fn!("__or_c2d_getImageData", cx_c2d_get_image_data);
    set_fn!("__or_c2d_putImageData", cx_c2d_put_image_data);
    set_fn!("__or_c2d_clipPath", cx_c2d_clip_path);
    // IPC
    set_fn!("__or_ipc_send", cx_ipc_send);
    // Misc
    set_fn!("__or_setDataValue", cx_set_data_value);
    set_fn!("__or_getViewportSize", cx_get_viewport_size);
    set_fn!("__or_dumpDoc", cx_dump_doc);
}

// ═══════════════════════════════════════════════════════════════════════════
// Console
// ═══════════════════════════════════════════════════════════════════════════

fn cx_log(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let level = v8_i32(&args, scope, 0);
    let msg = v8_str(scope, &args, 1);
    match level {
        0 => log::debug!("[JS] {}", msg),
        1 => log::info!("[JS] {}", msg),
        2 => log::warn!("[JS] {}", msg),
        3 => log::error!("[JS] {}", msg),
        _ => log::info!("[JS] {}", msg),
    }
    CONSOLE_BUFFER.with(|buf| buf.borrow_mut().push((level, msg)));
}

// ═══════════════════════════════════════════════════════════════════════════
// Performance
// ═══════════════════════════════════════════════════════════════════════════

fn cx_performance_now(scope: &mut v8::PinScope<'_, '_>, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    static EPOCH: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    let epoch = EPOCH.get_or_init(Instant::now);
    let now = epoch.elapsed().as_secs_f64() * 1000.0;
    rv.set(v8::Number::new(scope, now).into());
}

// ═══════════════════════════════════════════════════════════════════════════
// DOM
// ═══════════════════════════════════════════════════════════════════════════

fn cx_get_element_by_id(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let id = v8_str(scope, &args, 0);
    let nid = with_state(|st| {
        let reachable = get_reachable(st);
        for node in &st.document.nodes {
            if !reachable.get(node.id as usize).copied().unwrap_or(false) { continue; }
            if node.html_id.as_deref() == Some(id.as_str()) {
                return node.id as i32;
            }
        }
        -1i32
    });
    rv.set(v8::Integer::new(scope, nid).into());
}

fn cx_query_selector(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let selector = v8_str(scope, &args, 0);
    let nid = with_state(|st| {
        let reachable = get_reachable(st);
        for node in &st.document.nodes {
            if !reachable.get(node.id as usize).copied().unwrap_or(false) { continue; }
            if selector_matches_node(&selector, node, &st.document) {
                return node.id as i32;
            }
        }
        -1i32
    });
    rv.set(v8::Integer::new(scope, nid).into());
}

fn cx_query_selector_all(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let selector = v8_str(scope, &args, 0);
    let json = with_state(|st| {
        let reachable = get_reachable(st);
        let mut ids = Vec::new();
        for node in &st.document.nodes {
            if !reachable.get(node.id as usize).copied().unwrap_or(false) { continue; }
            if selector_matches_node(&selector, node, &st.document) {
                ids.push(node.id.to_string());
            }
        }
        format!("[{}]", ids.join(","))
    });
    let s = v8::String::new(scope, &json).unwrap();
    rv.set(s.into());
}

fn cx_create_element(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let tag = v8_str(scope, &args, 0);
    let nid = with_state(|st| {
        let mut node = crate::prd::node::PrdNode::container(0);
        node.tag = Some(tag.clone());
        if tag == "canvas" {
            node.kind = crate::prd::node::NodeKind::Canvas { width: 300, height: 150 };
        }
        let node_id = st.document.add_node(node);
        if tag == "canvas" {
            let cid = st.canvas_manager.create_canvas(300, 150);
            st.node_canvas_map.insert(node_id, cid);
            st.canvas_node_map.insert(cid, node_id);
            log::info!("[PRISM][DOM] createElement('canvas') → node={} canvas={}", node_id, cid);
        } else {
            log::info!("[PRISM][DOM] createElement('{}') → node={}", tag, node_id);
        }
        node_id as i32
    });
    rv.set(v8::Integer::new(scope, nid).into());
}

fn cx_get_text_content(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let text = with_state(|st| {
        if let Some(node) = st.document.get_node(nid) {
            if let NodeKind::Text { content } = &node.kind {
                return content.clone();
            }
            return collect_text_content(&st.document, nid);
        }
        String::new()
    });
    let s = v8::String::new(scope, &text).unwrap();
    rv.set(s.into());
}

fn cx_set_text_content(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let text = v8_str(scope, &args, 1);
    with_state(|st| {
        // If the target node is itself a Text node, just update its content.
        if let Some(node) = st.document.get_node(nid) {
            if matches!(node.kind, NodeKind::Text { .. }) {
                if let Some(node) = st.document.get_node_mut(nid) {
                    node.kind = NodeKind::Text { content: text };
                    st.layout_dirty = true;
                }
                return;
            }
        }

        // Fast path: if the container already has exactly one text child,
        // just update that child's content (avoids orphan node creation
        // on repeated textContent updates like FPS counters).
        if let Some(node) = st.document.get_node(nid) {
            if node.children.len() == 1 {
                let child_id = node.children[0];
                if let Some(child) = st.document.get_node(child_id) {
                    if matches!(child.kind, NodeKind::Text { .. }) {
                        if let Some(child) = st.document.get_node_mut(child_id) {
                            child.kind = NodeKind::Text { content: text };
                            st.layout_dirty = true;
                        }
                        return;
                    }
                }
            }
        }

        // For container nodes: clear children and add a new text child node,
        // preserving the parent's NodeKind (Container), styles, background, etc.
        let parent_style = match st.document.get_node(nid) {
            Some(n) => n.style.clone(),
            None => return,
        };

        // Free all descendant nodes so they don't remain orphaned in the
        // document arena (this was missing and caused old content to layer
        // on top of new content).
        free_descendants(st, nid);

        // Clear existing children.
        if let Some(node) = st.document.get_node_mut(nid) {
            node.children.clear();
        }

        // Invalidate the reachable-node cache since the tree changed.
        st.reachable_cache = None;

        // Create a new text child that inherits the parent's inheritable styles.
        let mut text_node = crate::prd::node::PrdNode::text(0, text);
        text_node.tag = Some("#text".to_string());
        text_node.style.color = parent_style.color;
        text_node.style.font_size = parent_style.font_size;
        text_node.style.font_family = parent_style.font_family;
        text_node.style.font_weight = parent_style.font_weight;
        text_node.style.line_height = parent_style.line_height;
        text_node.style.letter_spacing = parent_style.letter_spacing;
        text_node.style.text_align = parent_style.text_align;
        text_node.style.text_transform = parent_style.text_transform;

        let child_id = st.document.add_node(text_node);
        st.document.add_child(nid, child_id);
        st.layout_dirty = true;
    });
}

fn cx_set_inner_html(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let html = v8_str(scope, &args, 1);
    let scripts = with_state(|st| {
        log::info!("[PRISM][DOM] setInnerHTML: node={} html_len={}", nid, html.len());
        set_inner_html(st, nid, &html);
        // Drain deferred scripts collected from <script> blocks inside innerHTML
        std::mem::take(&mut st.deferred_scripts)
    });
    // Execute deferred scripts in the current V8 context
    for script_src in scripts {
        let src = v8::String::new(scope, &script_src).unwrap();
        let origin = v8::ScriptOrigin::new(scope, v8::String::new(scope, "<inline>").unwrap().into(), 0, 0, false, 0, None::<v8::Local<v8::Value>>, false, false, false, None);
        if let Some(compiled) = v8::Script::compile(scope, src, Some(&origin)) {
            if let None = compiled.run(scope) {
                log::warn!("[PRISM][JS] Deferred script from innerHTML failed to execute");
            }
        }
    }
}

fn cx_get_node_attribute(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let name = v8_str(scope, &args, 1);
    let result = with_state(|st| {
        st.document.get_node(nid)
            .and_then(|n| n.attributes.get(&name).cloned())
    });
    match result {
        Some(val) => rv.set(v8::String::new(scope, &val).unwrap().into()),
        None => rv.set(v8::null(scope).into()),
    }
}

fn cx_set_node_attribute(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let name = v8_str(scope, &args, 1);
    let value = v8_str(scope, &args, 2);
    with_state(|st| {
        let mut needs_restyle = false;
        if let Some(node) = st.document.get_node_mut(nid) {
            match name.as_str() {
                "id" => {
                    node.html_id = if value.is_empty() { None } else { Some(value.clone()) };
                    node.attributes.insert(name, value);
                    needs_restyle = true;
                }
                "class" => {
                    node.classes = value.split_whitespace().map(String::from).collect();
                    node.attributes.insert(name, value);
                    needs_restyle = true;
                }
                _ => {
                    node.attributes.insert(name, value);
                }
            }
            st.layout_dirty = true;
        }
        if needs_restyle {
            restyle_node(st, nid);
        }
    });
}

fn cx_class_list_op(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let op = v8_i32(&args, scope, 1);
    let class_name = v8_str(scope, &args, 2);
    let force = if args.length() > 3 {
        let v = args.get(3);
        if v.is_undefined() { None } else { Some(v.boolean_value(scope)) }
    } else {
        None
    };

    let result = with_state(|st| {
        let (result, changed) = if let Some(node) = st.document.get_node_mut(nid) {
            match op {
                0 => { // add
                    let changed = !node.classes.contains(&class_name);
                    if changed {
                        node.classes.push(class_name.clone());
                    }
                    (true, changed)
                }
                1 => { // remove
                    if let Some(pos) = node.classes.iter().position(|c| c == &class_name) {
                        node.classes.remove(pos);
                        (false, true)
                    } else {
                        (false, false)
                    }
                }
                2 => { // toggle
                    let has = node.classes.contains(&class_name);
                    let should_add = force.unwrap_or(!has);
                    if should_add && !has {
                        node.classes.push(class_name.clone());
                        (true, true)
                    } else if !should_add && has {
                        if let Some(pos) = node.classes.iter().position(|c| c == &class_name) {
                            node.classes.remove(pos);
                        }
                        (false, true)
                    } else {
                        (has, false)
                    }
                }
                3 => (node.classes.contains(&class_name), false), // contains
                _ => (false, false),
            }
        } else {
            (false, false)
        };
        if changed {
            restyle_node(st, nid);
        }
        result
    });
    rv.set(v8::Boolean::new(scope, result).into());
}

fn cx_get_style(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let prop = v8_str(scope, &args, 1);
    let val = with_state(|st| {
        st.document.get_node(nid)
            .map(|n| get_computed_style_value(&n.style, &prop))
            .unwrap_or_default()
    });
    rv.set(v8::String::new(scope, &val).unwrap().into());
}

fn cx_set_style(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let prop = v8_str(scope, &args, 1);
    let value = v8_str(scope, &args, 2);
    with_state(|st| {
        let vars_ptr = &st.css_variables as *const HashMap<String, String>;
        if let Some(node) = st.document.get_node_mut(nid) {
            // SAFETY: css_variables is not mutated during apply_property
            apply_property(&mut node.style, &prop, &value, unsafe { &*vars_ptr });
            // Persist JS-set styles into the inline `style` attribute so they
            // survive restyle passes (which rebuild from defaults → CSS → inline).
            let attr = node.attributes.entry("style".to_string()).or_default();
            // Append or replace the property in the inline style string.
            let css_prop = camel_to_kebab(&prop);
            let mut parts: Vec<String> = attr.split(';')
                .map(|s| s.trim().to_string())
                .filter(|s| {
                    if s.is_empty() { return false; }
                    if let Some((p, _)) = s.split_once(':') {
                        p.trim() != css_prop
                    } else {
                        true
                    }
                })
                .collect();
            parts.push(format!("{}: {}", css_prop, value));
            *attr = parts.join("; ");
            st.layout_dirty = true;
        }
    });
}

fn cx_get_computed_style_var(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let prop = v8_str(scope, &args, 0);
    let val = with_state(|st| {
        st.css_variables.get(&prop).cloned().unwrap_or_default()
    });
    rv.set(v8::String::new(scope, &val).unwrap().into());
}

fn cx_set_root_style_property(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let prop = v8_str(scope, &args, 0);
    let value = v8_str(scope, &args, 1);
    with_state(|st| {
        st.css_variables.insert(prop, value);
        st.layout_dirty = true;
    });
}

fn cx_get_node_tag(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let tag = with_state(|st| {
        st.document.get_node(nid)
            .and_then(|n| n.tag.as_ref().map(|t| t.to_uppercase()))
            .unwrap_or_else(|| "DIV".into())
    });
    rv.set(v8::String::new(scope, &tag).unwrap().into());
}

fn cx_get_node_children(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let json = with_state(|st| {
        st.document.get_node(nid)
            .map(|n| {
                let ids: Vec<String> = n.children.iter().map(|c| c.to_string()).collect();
                format!("[{}]", ids.join(","))
            })
            .unwrap_or_else(|| "[]".into())
    });
    rv.set(v8::String::new(scope, &json).unwrap().into());
}

fn cx_get_node_id(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let html_id = with_state(|st| {
        st.document.get_node(nid)
            .and_then(|n| n.html_id.clone())
            .unwrap_or_default()
    });
    rv.set(v8::String::new(scope, &html_id).unwrap().into());
}

fn cx_append_child(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let parent_nid = v8_i32(&args, scope, 0) as u32;
    let child_nid = v8_i32(&args, scope, 1) as u32;
    with_state(|st| {
        if let Some(old_parent) = st.document.find_parent(child_nid) {
            st.document.remove_child(old_parent, child_nid);
        }
        st.document.add_child(parent_nid, child_nid);
        st.reachable_cache = None;
        // Re-apply CSS rules on the moved node
        let rules_ptr = &st.css_rules as *const Vec<CssRule>;
        let vars_ptr = &st.css_variables as *const HashMap<String, String>;
        if let Some(node) = st.document.get_node_mut(child_nid) {
            // SAFETY: css_rules and css_variables are not mutated during apply_property
            let rules = unsafe { &*rules_ptr };
            let vars = unsafe { &*vars_ptr };
            for rule in rules {
                if simple_rule_matches(&rule.selector, node) {
                    for (prop, val) in &rule.declarations {
                        apply_property(&mut node.style, prop, val, vars);
                    }
                }
            }
        }
        st.layout_dirty = true;
    });
}

fn cx_remove_child(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let parent_nid = v8_i32(&args, scope, 0) as u32;
    let child_nid = v8_i32(&args, scope, 1) as u32;
    with_state(|st| {
        st.document.remove_child(parent_nid, child_nid);
        st.reachable_cache = None;
        st.layout_dirty = true;
    });
}

fn cx_get_parent_node(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let pid = with_state(|st| {
        st.document.find_parent(nid).map(|p| p as i32).unwrap_or(-1)
    });
    rv.set(v8::Integer::new(scope, pid).into());
}

fn cx_insert_before(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let parent_nid = v8_i32(&args, scope, 0) as u32;
    let new_nid = v8_i32(&args, scope, 1) as u32;
    let ref_nid = v8_i32(&args, scope, 2);
    with_state(|st| {
        if let Some(old_parent) = st.document.find_parent(new_nid) {
            st.document.remove_child(old_parent, new_nid);
        }
        if let Some(parent) = st.document.get_node_mut(parent_nid) {
            if ref_nid < 0 {
                parent.children.insert(0, new_nid);
            } else {
                let ref_id = ref_nid as u32;
                if let Some(pos) = parent.children.iter().position(|&c| c == ref_id) {
                    parent.children.insert(pos, new_nid);
                } else {
                    parent.children.push(new_nid);
                }
            }
        }
        st.layout_dirty = true;
    });
}

fn cx_get_node_client_size(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let size = with_state(|st| {
        if nid == st.document.root {
            return format!("{},{}", st.viewport_width, st.viewport_height);
        }

        if let Some(n) = st.document.get_node(nid) {
            let mut w = n.layout.rect.width.max(0.0);
            let mut h = n.layout.rect.height.max(0.0);

            if (w <= 0.0 || h <= 0.0) && matches!(n.kind, NodeKind::Canvas { .. }) {
                if let Some(cid) = st.node_canvas_map.get(&nid).copied() {
                    if let Some(canvas) = st.canvas_manager.buffers.get(&cid) {
                        w = canvas.width as f32;
                        h = canvas.height as f32;
                    }
                }
            }

            if w <= 0.0 {
                w = style_dimension_px(&n.style.width, st.viewport_width as f32).unwrap_or(0.0);
            }
            if h <= 0.0 {
                h = style_dimension_px(&n.style.height, st.viewport_height as f32).unwrap_or(0.0);
            }

            format!("{},{}", w.round() as i32, h.round() as i32)
        } else {
            "0,0".into()
        }
    });
    rv.set(v8::String::new(scope, &size).unwrap().into());
}

fn cx_get_node_rect(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let rect = with_state(|st| {
        if nid == st.document.root {
            return format!("0,0,{},{}", st.viewport_width, st.viewport_height);
        }
        st.document.get_node(nid)
            .map(|n| {
                format!(
                    "{},{},{},{}",
                    n.layout.rect.x.round() as i32,
                    n.layout.rect.y.round() as i32,
                    n.layout.rect.width.max(0.0).round() as i32,
                    n.layout.rect.height.max(0.0).round() as i32,
                )
            })
            .unwrap_or_else(|| "0,0,0,0".into())
    });
    rv.set(v8::String::new(scope, &rect).unwrap().into());
}

// ═══════════════════════════════════════════════════════════════════════════
// Canvas 2D
// ═══════════════════════════════════════════════════════════════════════════

fn cx_get_canvas_id(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let nid = v8_i32(&args, scope, 0) as u32;
    let cid = with_state(|st| {
        st.node_canvas_map.get(&nid).map(|&c| c as i32).unwrap_or(-1)
    });
    rv.set(v8::Integer::new(scope, cid).into());
}

fn cx_canvas_set_size(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let w = v8_i32(&args, scope, 1) as u32;
    let h = v8_i32(&args, scope, 2) as u32;
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.resize(w, h);
        }
    });
}

fn cx_canvas_get_size(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let size = with_state(|st| {
        st.canvas_manager.buffers.get(&cid)
            .map(|c| format!("{},{}", c.width, c.height))
            .unwrap_or_else(|| "0,0".into())
    });
    rv.set(v8::String::new(scope, &size).unwrap().into());
}

fn cx_c2d(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let cmd = v8_i32(&args, scope, 1);
    let a1 = v8_f32(&args, scope, 2);
    let a2 = v8_f32(&args, scope, 3);
    let a3 = v8_f32(&args, scope, 4);
    let a4 = v8_f32(&args, scope, 5);
    let a5 = v8_f32(&args, scope, 6);
    let a6 = v8_f32(&args, scope, 7);

    with_state(|st| {
        // Gradient-aware fill (cmd=9)
        if cmd == 9 {
            let uses_gradient = st.canvas_manager.buffers.get(&cid)
                .map(|c| c.uses_gradient_fill())
                .unwrap_or(false);
            if uses_gradient {
                st.canvas_manager.fill_path_with_gradient(cid);
            } else if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                canvas.fill();
            }
            return;
        }
        // Gradient-aware stroke (cmd=10)
        if cmd == 10 {
            let uses_gradient = st.canvas_manager.buffers.get(&cid)
                .map(|c| c.uses_gradient_stroke())
                .unwrap_or(false);
            if uses_gradient {
                st.canvas_manager.stroke_path_with_gradient(cid);
            } else if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                canvas.stroke();
            }
            return;
        }

        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            match cmd {
                1  => canvas.fill_rect(a1, a2, a3, a4),
                2  => canvas.stroke_rect(a1, a2, a3, a4),
                3  => canvas.clear_rect(a1, a2, a3, a4),
                4  => canvas.begin_path(),
                5  => canvas.close_path(),
                6  => canvas.move_to(a1, a2),
                7  => canvas.line_to(a1, a2),
                8  => canvas.arc(a1, a2, a3, a4, a5, a6 != 0.0),
                11 => canvas.save(),
                12 => canvas.restore(),
                13 => canvas.translate(a1, a2),
                14 => canvas.rotate(a1),
                15 => canvas.scale(a1, a2),
                16 => canvas.set_line_width(a1),
                17 => canvas.set_global_alpha(a1),
                18 => canvas.bezier_curve_to(a1, a2, a3, a4, a5, a6),
                19 => canvas.quadratic_curve_to(a1, a2, a3, a4),
                20 => canvas.set_transform(a1, a2, a3, a4, a5, a6),
                21 => canvas.reset_transform(),
                22 => canvas.clear(),
                _  => {}
            }
        }
    });
}

fn cx_c2d_set_fill_style(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let color_str = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(color) = parse_css_color(&color_str) {
            if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                canvas.set_fill_style_color(color);
            }
        }
    });
}

fn cx_c2d_set_stroke_style(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let color_str = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(color) = parse_css_color(&color_str) {
            if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                canvas.set_stroke_style_color(color);
            }
        }
    });
}

fn cx_c2d_set_fill_gradient(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let gid = v8_i32(&args, scope, 1) as u32;
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_fill_style_gradient(gid);
        }
    });
}

fn cx_c2d_set_stroke_gradient(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let gid = v8_i32(&args, scope, 1) as u32;
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_stroke_style_gradient(gid);
        }
    });
}

fn cx_c2d_set_fill_pattern(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let pid = v8_i32(&args, scope, 1) as u32;
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_fill_style_pattern(pid);
        }
    });
}

fn cx_c2d_set_blend_mode(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let mode = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_blend_mode(&mode);
        }
    });
}

fn cx_c2d_set_font(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let font = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_font(&font);
        }
    });
}

fn cx_c2d_set_text_align(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let align = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.text_align = align;
        }
    });
}

fn cx_c2d_set_text_baseline(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let baseline = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.text_baseline = baseline;
        }
    });
}

fn cx_c2d_set_line_cap(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let cap = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_line_cap(&cap);
        }
    });
}

fn cx_c2d_set_line_join(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let join = v8_str(scope, &args, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_line_join(&join);
        }
    });
}

fn cx_c2d_set_miter_limit(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let limit = v8_f32(&args, scope, 1);
    with_state(|st| {
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.set_miter_limit(limit);
        }
    });
}

fn cx_c2d_fill_text(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let text = v8_str(scope, &args, 1);
    let x = v8_f32(&args, scope, 2);
    let y = v8_f32(&args, scope, 3);
    with_state(|st| {
        // Split borrow: take font_system and swash_cache separately from buffers.
        let font_system = &mut st.canvas_manager.font_system;
        let swash_cache = &mut st.canvas_manager.swash_cache;
        if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
            canvas.fill_text(&text, x, y, font_system, swash_cache);
        }
    });
}

fn cx_c2d_draw_image(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let target_cid = v8_i32(&args, scope, 0) as u32;
    let source_cid = v8_i32(&args, scope, 1) as u32;
    let dx = v8_f32(&args, scope, 2);
    let dy = v8_f32(&args, scope, 3);
    let dw = v8_f32(&args, scope, 4);
    let dh = v8_f32(&args, scope, 5);
    with_state(|st| {
        st.canvas_manager.draw_canvas_to_canvas(target_cid, source_cid, dx, dy, dw, dh);
    });
}

fn cx_c2d_create_radial_gradient(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let x0 = v8_f32(&args, scope, 0);
    let y0 = v8_f32(&args, scope, 1);
    let r0 = v8_f32(&args, scope, 2);
    let x1 = v8_f32(&args, scope, 3);
    let y1 = v8_f32(&args, scope, 4);
    let r1 = v8_f32(&args, scope, 5);
    let gid = with_state(|st| {
        st.canvas_manager.create_gradient(GradientDef::Radial {
            x0, y0, r0, x1, y1, r1, stops: Vec::new(),
        }) as i32
    });
    rv.set(v8::Integer::new(scope, gid).into());
}

fn cx_c2d_create_linear_gradient(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let x0 = v8_f32(&args, scope, 0);
    let y0 = v8_f32(&args, scope, 1);
    let x1 = v8_f32(&args, scope, 2);
    let y1 = v8_f32(&args, scope, 3);
    let gid = with_state(|st| {
        st.canvas_manager.create_gradient(GradientDef::Linear {
            x0, y0, x1, y1, stops: Vec::new(),
        }) as i32
    });
    rv.set(v8::Integer::new(scope, gid).into());
}

fn cx_c2d_gradient_add_stop(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let gid = v8_i32(&args, scope, 0) as u32;
    let offset = v8_f32(&args, scope, 1);
    let color_str = v8_str(scope, &args, 2);
    with_state(|st| {
        if let Some(color) = parse_css_color(&color_str) {
            st.canvas_manager.add_gradient_stop(gid, offset, color);
        }
    });
}

fn cx_c2d_create_pattern(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let source_cid = v8_i32(&args, scope, 0);
    rv.set(v8::Integer::new(scope, source_cid).into());
}

fn cx_c2d_fill_rect_grad(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let x = v8_f32(&args, scope, 1);
    let y = v8_f32(&args, scope, 2);
    let w = v8_f32(&args, scope, 3);
    let h = v8_f32(&args, scope, 4);
    with_state(|st| {
        st.canvas_manager.fill_rect_with_gradient(cid, x, y, w, h);
    });
}

fn cx_c2d_fill_rect_pattern(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let x = v8_f32(&args, scope, 1);
    let y = v8_f32(&args, scope, 2);
    let w = v8_f32(&args, scope, 3);
    let h = v8_f32(&args, scope, 4);
    with_state(|st| {
        st.canvas_manager.fill_rect_with_pattern(cid, x, y, w, h);
    });
}

fn cx_c2d_get_image_data(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let x = v8_i32(&args, scope, 1);
    let y = v8_i32(&args, scope, 2);
    let w = v8_i32(&args, scope, 3);
    let h = v8_i32(&args, scope, 4);
    let json = with_state(|st| {
        match st.canvas_manager.get_image_data(cid, x, y, w, h) {
            Some((rw, rh, data)) => serde_json::json!({
                "width": rw,
                "height": rh,
                "data": data,
            }).to_string(),
            None => "{\"width\":0,\"height\":0,\"data\":[]}".to_string(),
        }
    });
    rv.set(v8::String::new(scope, &json).unwrap().into());
}

fn cx_c2d_put_image_data(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    let x = v8_i32(&args, scope, 1);
    let y = v8_i32(&args, scope, 2);
    let w = v8_i32(&args, scope, 3);
    let h = v8_i32(&args, scope, 4);
    let data_json = v8_str(scope, &args, 5);
    if let Ok(data) = serde_json::from_str::<Vec<u8>>(&data_json) {
        with_state(|st| {
            st.canvas_manager.put_image_data(cid, x, y, w, h, &data);
        });
    }
}

fn cx_c2d_clip_path(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let cid = v8_i32(&args, scope, 0) as u32;
    with_state(|st| {
        st.canvas_manager.clip_current_path(cid);
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// IPC
// ═══════════════════════════════════════════════════════════════════════════

fn cx_ipc_send(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let pipe_name = v8_str(scope, &args, 0);
    let request_json = v8_str(scope, &args, 1);

    let request: IpcRequest = match serde_json::from_str(&request_json) {
        Ok(r) => r,
        Err(e) => {
            let err_json = format!(r#"{{"ok":false,"error":"Parse error: {}"}}"#, e);
            rv.set(v8::String::new(scope, &err_json).unwrap().into());
            return;
        }
    };

    match send_ipc_request_to(&pipe_name, request) {
        Ok(resp) => {
            let json = serde_json::to_string(&resp).unwrap_or_else(|_| {
                r#"{"ok":false,"error":"Serialize error"}"#.to_string()
            });
            rv.set(v8::String::new(scope, &json).unwrap().into());
        }
        Err(e) => {
            let err_json = format!(r#"{{"ok":false,"error":"{}"}}"#, e.replace('"', "'"));
            rv.set(v8::String::new(scope, &err_json).unwrap().into());
        }
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Misc
// ═══════════════════════════════════════════════════════════════════════════

fn cx_set_data_value(scope: &mut v8::PinScope<'_, '_>, args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    let key = v8_str(scope, &args, 0);
    let value = v8_str(scope, &args, 1);
    with_state(|st| {
        st.data_values.insert(key, value);
    });
}

fn cx_get_viewport_size(scope: &mut v8::PinScope<'_, '_>, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let size = with_state(|st| {
        format!("{},{}", st.viewport_width, st.viewport_height)
    });
    rv.set(v8::String::new(scope, &size).unwrap().into());
}

fn cx_dump_doc(_scope: &mut v8::PinScope<'_, '_>, _args: v8::FunctionCallbackArguments, _rv: v8::ReturnValue<v8::Value>) {
    with_state(|st| {
        log::warn!("[PRISM][DEBUG] === Document dump: {} nodes ===", st.document.nodes.len());
        for node in &st.document.nodes {
            let tag = node.tag.as_deref().unwrap_or("(none)");
            let id = node.html_id.as_deref().unwrap_or("");
            let classes = node.classes.join(" ");
            let kind = match &node.kind {
                NodeKind::Container => "Container",
                NodeKind::Text { content } => {
                    log::warn!("[PRISM][DEBUG]   node {} tag={} id={} class='{}' kind=Text text='{}'",
                        node.id, tag, id, classes, &content[..content.len().min(60)]);
                    continue;
                },
                NodeKind::Canvas { .. } => "Canvas",
                _ => "Other",
            };
            let children_str: Vec<String> = node.children.iter().map(|c| c.to_string()).collect();
            let has_canvas = st.node_canvas_map.contains_key(&node.id);
            log::warn!("[PRISM][DEBUG]   node {} tag={} id={} class='{}' kind={} children=[{}] canvas={}",
                node.id, tag, id, classes, kind, children_str.join(","), has_canvas);
        }
        log::warn!("[PRISM][DEBUG] === End document dump ===");
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper functions (engine-agnostic)
// ═══════════════════════════════════════════════════════════════════════════

fn selector_matches_node(selector: &str, node: &crate::prd::node::PrdNode, _doc: &PrdDocument) -> bool {
    let sel = selector.trim();
    if sel.starts_with('#') {
        return node.html_id.as_deref() == Some(&sel[1..]);
    }
    if sel.starts_with('.') {
        let cls = &sel[1..];
        return node.classes.iter().any(|c| c == cls);
    }
    if let Some(tag) = &node.tag {
        return tag == sel;
    }
    false
}

fn collect_text_content(doc: &PrdDocument, node_id: NodeId) -> String {
    let mut text = String::new();
    if let Some(node) = doc.get_node(node_id) {
        if let NodeKind::Text { content } = &node.kind {
            text.push_str(content);
        }
        for &child_id in &node.children {
            text.push_str(&collect_text_content(doc, child_id));
        }
    }
    text
}

fn free_descendants(st: &mut SharedState, node_id: NodeId) {
    let children: Vec<NodeId> = st.document.get_node(node_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();

    for child_id in children {
        free_descendants(st, child_id);
        if let Some(cid) = st.node_canvas_map.remove(&child_id) {
            st.canvas_node_map.remove(&cid);
        }
        if let Some(node) = st.document.get_node_mut(child_id) {
            *node = crate::prd::node::PrdNode::container(child_id);
        }
        st.document.free_list.push(child_id);
    }
}

fn set_inner_html(st: &mut SharedState, node_id: NodeId, html: &str) {
    free_descendants(st, node_id);

    if let Some(node) = st.document.get_node_mut(node_id) {
        node.children.clear();
        if matches!(node.kind, NodeKind::Text { .. }) {
            node.kind = NodeKind::Container;
        }
    }

    if !html.trim().is_empty() {
        add_html_children(st, node_id, html);
    }

    st.reachable_cache = None;
    st.layout_dirty = true;
}

fn decode_base64(input: &str) -> Option<Vec<u8>> {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut lut = [255u8; 256];
    for (i, &c) in TABLE.iter().enumerate() {
        lut[c as usize] = i as u8;
    }
    let input: Vec<u8> = input.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    let mut out = Vec::with_capacity(input.len() * 3 / 4);
    for chunk in input.chunks(4) {
        let mut buf = [0u8; 4];
        let mut valid = 0usize;
        for (i, &b) in chunk.iter().enumerate() {
            if b == b'=' { break; }
            if lut[b as usize] == 255 { return None; }
            buf[i] = lut[b as usize];
            valid += 1;
        }
        if valid >= 2 { out.push((buf[0] << 2) | (buf[1] >> 4)); }
        if valid >= 3 { out.push((buf[1] << 4) | (buf[2] >> 2)); }
        if valid >= 4 { out.push((buf[2] << 6) | buf[3]); }
    }
    Some(out)
}

fn decode_data_url_image(src: &str) -> Option<(u32, u32, Vec<u8>)> {
    let rest = src.strip_prefix("data:")?;
    let (_header, b64) = rest.split_once(',')?;
    let raw = decode_base64(b64)?;
    let img = image::load_from_memory(&raw).ok()?;
    let rgba = img.to_rgba8();
    let (w, h) = rgba.dimensions();
    if w == 0 || h == 0 { return None; }
    let mut pixels = rgba.into_raw();
    for chunk in pixels.chunks_exact_mut(4) {
        let a = chunk[3] as u32;
        if a < 255 {
            chunk[0] = ((chunk[0] as u32 * a + 128) / 255) as u8;
            chunk[1] = ((chunk[1] as u32 * a + 128) / 255) as u8;
            chunk[2] = ((chunk[2] as u32 * a + 128) / 255) as u8;
        }
    }
    Some((w, h, pixels))
}

/// Extract an attribute value from raw SVG markup by name.
fn extract_svg_attr<'a>(svg: &'a str, attr: &str) -> Option<&'a str> {
    let pattern = format!("{}=\"", attr);
    let start = svg.find(&pattern)? + pattern.len();
    let end = start + svg[start..].find('"')?;
    Some(&svg[start..end])
}

/// Extract width and height from an SVG viewBox attribute.
fn extract_svg_viewbox(svg: &str) -> (u32, u32) {
    // Try both casings since original SVG may have camelCase viewBox.
    let vb = extract_svg_attr(svg, "viewBox")
        .or_else(|| extract_svg_attr(svg, "viewbox"));
    match vb {
        Some(val) => {
            let parts: Vec<f32> = val.split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            if parts.len() == 4 {
                (parts[2].ceil() as u32, parts[3].ceil() as u32)
            } else {
                (0, 0)
            }
        }
        None => (0, 0),
    }
}

fn add_html_children(st: &mut SharedState, parent_id: NodeId, html: &str) {
    let bytes = html.as_bytes();
    let mut pos = 0usize;
    let mut deferred_scripts: Vec<String> = Vec::new();

    let mut node_stack: Vec<NodeId> = vec![parent_id];
    let mut ancestor_stack: Vec<Vec<AncestorInfo>> = vec![collect_ancestor_chain(&st.document, parent_id)];

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            if pos + 3 < bytes.len() && &html[pos..pos + 4] == "<!--" {
                if let Some(end) = html[pos + 4..].find("-->") {
                    pos += 4 + end + 3;
                    continue;
                }
                break;
            }

            if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
                while pos < bytes.len() && bytes[pos] != b'>' { pos += 1; }
                if pos < bytes.len() { pos += 1; }
                if node_stack.len() > 1 {
                    node_stack.pop();
                    ancestor_stack.pop();
                }
                continue;
            }

            // Peek the tag name before consuming
            let tag_peek_start = pos + 1;
            let mut peek = tag_peek_start;
            while peek < bytes.len() && !bytes[peek].is_ascii_whitespace() && bytes[peek] != b'>' && bytes[peek] != b'/' {
                peek += 1;
            }
            let tag_peek = html[tag_peek_start..peek].trim().to_lowercase();

            // Handle <style> — extract CSS, parse into rules, don't render as visible node.
            if tag_peek == "style" {
                // Skip past the opening <style ...> tag
                while pos < bytes.len() && bytes[pos] != b'>' { pos += 1; }
                if pos < bytes.len() { pos += 1; }
                // Find matching </style>
                let lower_rest = html[pos..].to_lowercase();
                if let Some(end) = lower_rest.find("</style>") {
                    let css_text = &html[pos..pos + end];
                    let new_rules = crate::compiler::css::parse_css(css_text);
                    st.css_rules.extend(new_rules);
                    pos += end + 8; // skip past </style>
                }
                continue;
            }

            // Handle <script> — extract JS, queue for deferred execution.
            if tag_peek == "script" {
                // Skip past the opening <script ...> tag
                while pos < bytes.len() && bytes[pos] != b'>' { pos += 1; }
                if pos < bytes.len() { pos += 1; }
                // Find matching </script>
                let lower_rest = html[pos..].to_lowercase();
                if let Some(end) = lower_rest.find("</script>") {
                    let js_text = html[pos..pos + end].to_string();
                    if !js_text.trim().is_empty() {
                        deferred_scripts.push(js_text);
                    }
                    pos += end + 9; // skip past </script>
                }
                continue;
            }

            // Handle <svg> — capture raw markup and rasterize to image.
            if tag_peek == "svg" {
                // Find the start of the `<svg` tag in the original HTML.
                let svg_start = pos - tag_peek.len() - 1;
                let lower_rest = html[pos..].to_lowercase();
                if let Some(end) = lower_rest.find("</svg>") {
                    let svg_end = pos + end + 6;
                    let raw_svg = &html[svg_start..svg_end];

                    // Ensure xmlns is present for resvg.
                    let svg_markup = if !raw_svg.contains("xmlns") {
                        raw_svg.replacen("<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"", 1)
                    } else {
                        raw_svg.to_string()
                    };

                    // Resolve currentColor from the parent node.
                    let current_color = node_stack.last()
                        .and_then(|pid| st.document.get_node(*pid))
                        .map(|n| n.style.color)
                        .unwrap_or(crate::prd::value::Color::WHITE);
                    let color_hex = current_color.to_hex_string();
                    let svg_markup = svg_markup.replace("currentColor", &color_hex);

                    // Parse width/height attributes for layout sizing.
                    let target_w = extract_svg_attr(&svg_markup, "width");
                    let target_h = extract_svg_attr(&svg_markup, "height");
                    let tw: u32 = target_w.and_then(|v| v.parse().ok()).unwrap_or(0);
                    let th: u32 = target_h.and_then(|v| v.parse().ok()).unwrap_or(0);

                    // Parse viewBox as fallback dimensions.
                    let (vb_w, vb_h) = extract_svg_viewbox(&svg_markup);

                    let scale = 2u32;
                    let raster_w = if tw > 0 { tw * scale } else if vb_w > 0 { vb_w * scale } else { 64 };
                    let raster_h = if th > 0 { th * scale } else if vb_h > 0 { vb_h * scale } else { 64 };

                    if let Some((rgba, w, h)) = crate::compiler::html::rasterize_svg(&svg_markup, raster_w, raster_h) {
                        let name = format!("svg_dyn_{}", st.document.assets.images.len());
                        let asset_idx = st.document.assets.add_raw_image(name, w, h, rgba);

                        let mut node = crate::prd::node::PrdNode::container(0);
                        node.tag = Some("svg".to_string());
                        node.kind = NodeKind::Image {
                            asset_index: asset_idx,
                            fit: crate::prd::node::ImageFit::Contain,
                        };
                        node.style.display = crate::prd::style::Display::InlineBlock;
                        node.style.background = crate::prd::style::Background::Image { asset_index: asset_idx };

                        // Set dimensions from attributes or viewBox.
                        let dim_w = if tw > 0 { tw } else { vb_w };
                        let dim_h = if th > 0 { th } else { vb_h };
                        if dim_w > 0 {
                            node.style.width = crate::prd::value::Dimension::Px(dim_w as f32);
                        }
                        if dim_h > 0 {
                            node.style.height = crate::prd::value::Dimension::Px(dim_h as f32);
                        }

                        // Inherit styles from parent.
                        if let Some(parent_node) = node_stack.last().and_then(|pid| st.document.get_node(*pid)) {
                            node.style.color = parent_node.style.color;
                            node.style.font_size = parent_node.style.font_size;
                            node.style.font_family = parent_node.style.font_family.clone();
                            node.style.font_weight = parent_node.style.font_weight;
                        }

                        let parent = *node_stack.last().unwrap_or(&parent_id);
                        let child_id = st.document.add_node(node);
                        st.document.add_child(parent, child_id);
                        st.assets_dirty = true;
                    }

                    pos = svg_end;
                    continue;
                }
            }

            pos += 1;
            let tag_start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                pos += 1;
            }
            let tag = html[tag_start..pos].trim().to_lowercase();
            if tag.is_empty() {
                while pos < bytes.len() && bytes[pos] != b'>' { pos += 1; }
                if pos < bytes.len() { pos += 1; }
                continue;
            }

            let mut classes = Vec::new();
            let mut html_id = None;
            let mut inline_style = String::new();
            let mut attributes = HashMap::new();

            loop {
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
                if pos >= bytes.len() || bytes[pos] == b'>' || bytes[pos] == b'/' { break; }

                let attr_start = pos;
                while pos < bytes.len()
                    && bytes[pos] != b'='
                    && !bytes[pos].is_ascii_whitespace()
                    && bytes[pos] != b'>'
                    && bytes[pos] != b'/'
                {
                    pos += 1;
                }
                let attr_name = html[attr_start..pos].to_lowercase();

                let attr_value = if pos < bytes.len() && bytes[pos] == b'=' {
                    pos += 1;
                    if pos < bytes.len() && (bytes[pos] == b'"' || bytes[pos] == b'\'') {
                        let quote = bytes[pos];
                        pos += 1;
                        let val_start = pos;
                        while pos < bytes.len() && bytes[pos] != quote { pos += 1; }
                        let val = html[val_start..pos].to_string();
                        if pos < bytes.len() { pos += 1; }
                        val
                    } else {
                        let val_start = pos;
                        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' {
                            pos += 1;
                        }
                        html[val_start..pos].to_string()
                    }
                } else {
                    String::new()
                };

                match attr_name.as_str() {
                    "class" => classes = attr_value.split_whitespace().map(String::from).collect(),
                    "id" => html_id = Some(attr_value.clone()),
                    "style" => inline_style = attr_value.clone(),
                    _ => { attributes.insert(attr_name, attr_value); }
                }
            }

            let self_closing = pos < bytes.len() && bytes[pos] == b'/';
            if self_closing { pos += 1; }
            if pos < bytes.len() && bytes[pos] == b'>' { pos += 1; }

            let img_data = if tag == "img" {
                attributes.get("src").and_then(|s| decode_data_url_image(s))
            } else {
                None
            };

            let mut node = crate::prd::node::PrdNode::container(0);
            node.tag = Some(tag.clone());
            node.html_id = html_id;
            node.classes = classes;
            node.attributes = attributes;
            if let Some((iw, ih, _)) = &img_data {
                node.kind = NodeKind::Canvas { width: *iw, height: *ih };
            } else if tag == "canvas" {
                node.kind = NodeKind::Canvas { width: 300, height: 150 };
            }

            apply_dynamic_tag_defaults(&mut node);

            if let Some(parent) = node_stack.last().and_then(|pid| st.document.get_node(*pid)) {
                node.style.color = parent.style.color;
                node.style.font_size = parent.style.font_size;
                node.style.font_family = parent.style.font_family.clone();
                node.style.font_weight = parent.style.font_weight;
                node.style.letter_spacing = parent.style.letter_spacing;
                node.style.line_height = parent.style.line_height;
                node.style.text_align = parent.style.text_align;
                node.style.text_transform = parent.style.text_transform;
            }

            let ancestors = ancestor_stack.last().cloned().unwrap_or_default();
            for rule in &st.css_rules {
                if compound_selector_matches(&rule.compound_selectors, &node, &node.html_id, &ancestors) {
                    if let Some(ref pseudo) = rule.pseudo_class {
                        let vars = &st.css_variables;
                        match pseudo.as_str() {
                            "hover" => {
                                for (prop, val) in &rule.declarations {
                                    let resolved = crate::compiler::css::resolve_var_pub(val, vars);
                                    node.hover_style.push((prop.clone(), resolved));
                                }
                            }
                            "active" => {
                                for (prop, val) in &rule.declarations {
                                    let resolved = crate::compiler::css::resolve_var_pub(val, vars);
                                    node.active_style.push((prop.clone(), resolved));
                                }
                            }
                            "focus" | "focus-visible" | "focus-within" => {
                                for (prop, val) in &rule.declarations {
                                    let resolved = crate::compiler::css::resolve_var_pub(val, vars);
                                    node.focus_style.push((prop.clone(), resolved));
                                }
                            }
                            _ => {}
                        }
                    } else {
                        for (prop, val) in &rule.declarations {
                            apply_property(&mut node.style, prop, val, &st.css_variables);
                        }
                    }
                }
            }

            if !inline_style.is_empty() {
                for decl in inline_style.split(';') {
                    let decl = decl.trim();
                    if let Some((prop, val)) = decl.split_once(':') {
                        apply_property(&mut node.style, prop.trim(), val.trim(), &st.css_variables);
                    }
                }
            }

            // Extract event bindings from data-action attributes (same as compiler).
            extract_runtime_event_bindings(&mut node);

            let parent = *node_stack.last().unwrap_or(&parent_id);
            let child_id = st.document.add_node(node);
            st.document.add_child(parent, child_id);

            if tag == "canvas" {
                let cid = st.canvas_manager.create_canvas(300, 150);
                st.node_canvas_map.insert(child_id, cid);
                st.canvas_node_map.insert(cid, child_id);
            }

            if let Some((img_w, img_h, pixels)) = img_data {
                let cid = st.canvas_manager.create_canvas(img_w, img_h);
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    if let Some(pm) = tiny_skia::Pixmap::from_vec(
                        pixels,
                        tiny_skia::IntSize::from_wh(img_w, img_h).unwrap(),
                    ) {
                        canvas.pixmap = pm;
                    }
                    canvas.dirty = true;
                }
                st.node_canvas_map.insert(child_id, cid);
                st.canvas_node_map.insert(cid, child_id);
            }

            let void_tags = ["img", "br", "hr", "input", "meta", "link", "source", "path", "line", "circle", "rect", "polyline", "ellipse", "polygon"];
            if !self_closing && !void_tags.contains(&tag.as_str()) {
                let mut child_ancestors = ancestors;
                child_ancestors.push(AncestorInfo {
                    tag: st.document.get_node(child_id).and_then(|n| n.tag.clone()),
                    classes: st.document.get_node(child_id).map(|n| n.classes.clone()).unwrap_or_default(),
                    html_id: st.document.get_node(child_id).and_then(|n| n.html_id.clone()),
                });
                node_stack.push(child_id);
                ancestor_stack.push(child_ancestors);
            }
        } else {
            let text_start = pos;
            while pos < bytes.len() && bytes[pos] != b'<' { pos += 1; }
            let raw_text = html[text_start..pos].trim();
            if raw_text.is_empty() { continue; }
            let text = decode_html_entities(raw_text);

            let parent = *node_stack.last().unwrap_or(&parent_id);
            let mut text_node = crate::prd::node::PrdNode::text(0, &text);
            if let Some(parent_node) = st.document.get_node(parent) {
                text_node.style.color = parent_node.style.color;
                text_node.style.font_size = parent_node.style.font_size;
                text_node.style.font_family = parent_node.style.font_family.clone();
                text_node.style.font_weight = parent_node.style.font_weight;
                text_node.style.letter_spacing = parent_node.style.letter_spacing;
                text_node.style.line_height = parent_node.style.line_height;
                text_node.style.text_align = parent_node.style.text_align;
                text_node.style.text_transform = parent_node.style.text_transform;
            }
            let text_id = st.document.add_node(text_node);
            st.document.add_child(parent, text_id);
        }
    }

    // Execute deferred <script> blocks after all HTML has been added to the DOM.
    if !deferred_scripts.is_empty() {
        st.deferred_scripts.extend(deferred_scripts);
    }
}

/// Re-apply CSS rules to a node after its classes have changed.
/// Resets the node's style to defaults + tag defaults + parent inheritance,
/// then re-evaluates all CSS rules (including pseudo-class rules).
fn restyle_node(st: &mut SharedState, node_id: NodeId) {
    use crate::prd::style::ComputedStyle;
    use crate::compiler::css::resolve_var_pub;

    let ancestors = collect_ancestor_chain(&st.document, node_id);

    // Collect parent inherited styles
    let parent_id = st.document.find_parent(node_id);
    let (parent_color, parent_font_size, parent_font_family, parent_font_weight,
         parent_letter_spacing, parent_line_height, parent_text_align, parent_text_transform) =
        parent_id
            .and_then(|pid| st.document.get_node(pid))
            .map(|p| (
                p.style.color,
                p.style.font_size,
                p.style.font_family.clone(),
                p.style.font_weight,
                p.style.letter_spacing,
                p.style.line_height,
                p.style.text_align,
                p.style.text_transform,
            ))
            .unwrap_or_default();

    // Collect inline style string before mutating
    let inline_style = st.document.get_node(node_id)
        .and_then(|n| n.attributes.get("style").cloned())
        .unwrap_or_default();

    if let Some(node) = st.document.get_node_mut(node_id) {
        // Reset to defaults
        node.style = ComputedStyle::default();
        node.hover_style.clear();
        node.active_style.clear();
        node.focus_style.clear();

        // Apply tag defaults
        apply_dynamic_tag_defaults(node);

        // Inherit from parent
        node.style.color = parent_color;
        node.style.font_size = parent_font_size;
        node.style.font_family = parent_font_family;
        node.style.font_weight = parent_font_weight;
        node.style.letter_spacing = parent_letter_spacing;
        node.style.line_height = parent_line_height;
        node.style.text_align = parent_text_align;
        node.style.text_transform = parent_text_transform;

        // Re-apply all matching CSS rules (including pseudo-class rules)
        for rule in &st.css_rules {
            if compound_selector_matches(&rule.compound_selectors, node, &node.html_id.clone(), &ancestors) {
                if let Some(ref pseudo) = rule.pseudo_class {
                    match pseudo.as_str() {
                        "hover" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.hover_style.push((prop.clone(), resolved));
                            }
                        }
                        "active" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.active_style.push((prop.clone(), resolved));
                            }
                        }
                        "focus" | "focus-visible" | "focus-within" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.focus_style.push((prop.clone(), resolved));
                            }
                        }
                        _ => {}
                    }
                } else {
                    for (prop, val) in &rule.declarations {
                        apply_property(&mut node.style, prop, val, &st.css_variables);
                    }
                }
            }
        }

        // Re-apply inline styles (highest priority)
        if !inline_style.is_empty() {
            for decl in inline_style.split(';') {
                let decl = decl.trim();
                if let Some((prop, val)) = decl.split_once(':') {
                    apply_property(&mut node.style, prop.trim(), val.trim(), &st.css_variables);
                }
            }
        }
    }

    // Recursively restyle descendants so that descendant selectors
    // (e.g. `.parent.active .child`) are re-evaluated when the parent's
    // classes change.
    let children: Vec<NodeId> = st.document.get_node(node_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    for child_id in children {
        restyle_subtree(st, child_id);
    }

    st.layout_dirty = true;
}

/// Recursively restyle a node and all its descendants. Used after a parent
/// class change to ensure descendant selectors are re-evaluated.
fn restyle_subtree(st: &mut SharedState, node_id: NodeId) {
    use crate::prd::style::ComputedStyle;
    use crate::compiler::css::resolve_var_pub;

    let ancestors = collect_ancestor_chain(&st.document, node_id);

    let parent_id = st.document.find_parent(node_id);
    let (parent_color, parent_font_size, parent_font_family, parent_font_weight,
         parent_letter_spacing, parent_line_height, parent_text_align, parent_text_transform) =
        parent_id
            .and_then(|pid| st.document.get_node(pid))
            .map(|p| (
                p.style.color,
                p.style.font_size,
                p.style.font_family.clone(),
                p.style.font_weight,
                p.style.letter_spacing,
                p.style.line_height,
                p.style.text_align,
                p.style.text_transform,
            ))
            .unwrap_or_default();

    let inline_style = st.document.get_node(node_id)
        .and_then(|n| n.attributes.get("style").cloned())
        .unwrap_or_default();

    if let Some(node) = st.document.get_node_mut(node_id) {
        node.style = ComputedStyle::default();
        node.hover_style.clear();
        node.active_style.clear();
        node.focus_style.clear();

        apply_dynamic_tag_defaults(node);

        node.style.color = parent_color;
        node.style.font_size = parent_font_size;
        node.style.font_family = parent_font_family;
        node.style.font_weight = parent_font_weight;
        node.style.letter_spacing = parent_letter_spacing;
        node.style.line_height = parent_line_height;
        node.style.text_align = parent_text_align;
        node.style.text_transform = parent_text_transform;

        for rule in &st.css_rules {
            if compound_selector_matches(&rule.compound_selectors, node, &node.html_id.clone(), &ancestors) {
                if let Some(ref pseudo) = rule.pseudo_class {
                    match pseudo.as_str() {
                        "hover" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.hover_style.push((prop.clone(), resolved));
                            }
                        }
                        "active" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.active_style.push((prop.clone(), resolved));
                            }
                        }
                        "focus" | "focus-visible" | "focus-within" => {
                            for (prop, val) in &rule.declarations {
                                let resolved = resolve_var_pub(val, &st.css_variables);
                                node.focus_style.push((prop.clone(), resolved));
                            }
                        }
                        _ => {}
                    }
                } else {
                    for (prop, val) in &rule.declarations {
                        apply_property(&mut node.style, prop, val, &st.css_variables);
                    }
                }
            }
        }

        if !inline_style.is_empty() {
            for decl in inline_style.split(';') {
                let decl = decl.trim();
                if let Some((prop, val)) = decl.split_once(':') {
                    apply_property(&mut node.style, prop.trim(), val.trim(), &st.css_variables);
                }
            }
        }
    }

    let children: Vec<NodeId> = st.document.get_node(node_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();
    for child_id in children {
        restyle_subtree(st, child_id);
    }
}

fn collect_ancestor_chain(doc: &PrdDocument, node_id: NodeId) -> Vec<AncestorInfo> {
    let mut chain = Vec::new();
    let mut cursor = Some(node_id);
    while let Some(id) = cursor {
        if let Some(n) = doc.get_node(id) {
            chain.push(AncestorInfo {
                tag: n.tag.clone(),
                classes: n.classes.clone(),
                html_id: n.html_id.clone(),
            });
            cursor = doc.find_parent(id);
        } else {
            break;
        }
    }
    chain.reverse();
    chain
}

/// Extract event bindings from data-action/data-ns/data-cmd attributes on
/// dynamically inserted nodes (innerHTML). Mirrors the compiler logic.
fn extract_runtime_event_bindings(node: &mut crate::prd::node::PrdNode) {
    use crate::prd::node::{EventAction, EventBinding};

    let action_type = match node.attributes.get("data-action") {
        Some(a) => a.clone(),
        None => {
            // data-navigate shorthand
            if let Some(target) = node.attributes.get("data-navigate").cloned() {
                node.events.push(EventBinding {
                    event: "click".to_string(),
                    action: EventAction::Navigate { scene_id: target },
                });
            }
            return;
        }
    };

    let action = match action_type.as_str() {
        "navigate" => {
            let target = node.attributes.get("data-target").cloned().unwrap_or_default();
            EventAction::Navigate { scene_id: target }
        }
        "ipc" => {
            let ns = node.attributes.get("data-ns").cloned().unwrap_or_default();
            let cmd = node.attributes.get("data-cmd").cloned().unwrap_or_default();
            let args = node.attributes.get("data-args")
                .and_then(|a| serde_json::from_str(a).ok());
            EventAction::IpcCommand { ns, cmd, args }
        }
        "toggle-class" => {
            let class = node.attributes.get("data-class").cloned().unwrap_or_default();
            let target_id = node.attributes.get("data-target").cloned().unwrap_or_default();
            EventAction::ToggleClass { target: 0, class, target_html_id: target_id }
        }
        "window-close" => EventAction::WindowClose,
        "window-minimize" => EventAction::WindowMinimize,
        "window-maximize" => EventAction::WindowMaximize,
        "window-drag" => EventAction::WindowDrag,
        _ => {
            EventAction::IpcCommand { ns: String::new(), cmd: action_type, args: None }
        }
    };

    let event_type = node.attributes.get("data-event").cloned().unwrap_or_else(|| "click".to_string());
    node.events.push(EventBinding { event: event_type, action });

    // data-navigate shorthand (can coexist)
    if let Some(target) = node.attributes.get("data-navigate").cloned() {
        node.events.push(EventBinding {
            event: "click".to_string(),
            action: EventAction::Navigate { scene_id: target },
        });
    }
}

fn apply_dynamic_tag_defaults(node: &mut crate::prd::node::PrdNode) {
    use crate::prd::style::{Display, FlexDirection, FontWeight};

    if let Some(tag) = node.tag.as_deref() {
        match tag {
            "span" | "a" | "label" | "code" | "small" => {
                node.style.display = Display::Flex;
                node.style.flex_direction = FlexDirection::Row;
            }
            "strong" | "b" => {
                node.style.display = Display::Flex;
                node.style.flex_direction = FlexDirection::Row;
                node.style.font_weight = FontWeight(700);
            }
            "em" | "i" => {
                node.style.display = Display::Flex;
                node.style.flex_direction = FlexDirection::Row;
            }
            "h1" => { node.style.font_size = 32.0; node.style.font_weight = FontWeight(700); }
            "h2" => { node.style.font_size = 24.0; node.style.font_weight = FontWeight(700); }
            "h3" => { node.style.font_size = 18.72; node.style.font_weight = FontWeight(700); }
            "h4" => { node.style.font_size = 16.0; node.style.font_weight = FontWeight(700); }
            "h5" => { node.style.font_size = 13.28; node.style.font_weight = FontWeight(700); }
            "h6" => { node.style.font_size = 10.72; node.style.font_weight = FontWeight(700); }
            "data-bind" => {
                node.style.display = Display::InlineBlock;
                if node.classes.iter().any(|c| c == "val") {
                    node.style.flex_grow = 1.0;
                }
            }
            "data-bar" => {
                node.style.display = Display::Block;
            }
            "canvas" => {
                node.style.display = Display::Block;
            }
            _ => {}
        }
    }
}

fn simple_rule_matches(selector: &str, node: &crate::prd::node::PrdNode) -> bool {
    let parts: Vec<&str> = selector.split_whitespace().collect();
    if parts.is_empty() { return false; }
    let last = parts.last().unwrap();
    if *last == "*" { return true; }
    if last.starts_with('.') {
        let cls = &last[1..];
        return node.classes.iter().any(|c| c == cls);
    }
    if last.starts_with('#') {
        let id = &last[1..];
        return node.html_id.as_deref() == Some(id);
    }
    if let Some(tag) = &node.tag {
        return tag == last;
    }
    false
}

fn get_computed_style_value(style: &crate::prd::style::ComputedStyle, prop: &str) -> String {
    use crate::prd::style::Display;
    match prop {
        "display" => match style.display {
            Display::None => "none".into(),
            Display::Block => "block".into(),
            Display::Flex => "flex".into(),
            Display::InlineFlex => "inline-flex".into(),
            Display::InlineBlock => "inline-block".into(),
            Display::Inline => "inline".into(),
            Display::Grid => "grid".into(),
            Display::InlineGrid => "inline-grid".into(),
        },
        "opacity" => format!("{}", style.opacity),
        "background" | "background-color" | "backgroundColor" => {
            match &style.background {
                crate::prd::style::Background::Solid(c) => {
                    format!(
                        "rgba({},{},{},{})",
                        (c.r * 255.0) as u8,
                        (c.g * 255.0) as u8,
                        (c.b * 255.0) as u8,
                        c.a,
                    )
                }
                _ => String::new(),
            }
        }
        "font-size" | "fontSize" => format!("{}px", style.font_size),
        "font-family" | "fontFamily" => style.font_family.clone(),
        "padding" => {
            use crate::prd::value::Dimension;
            match style.padding.top {
                Dimension::Px(v) => format!("{}px", v),
                Dimension::Percent(v) => format!("{}%", v),
                _ => String::new(),
            }
        },
        "overflow" => match style.overflow {
            crate::prd::style::Overflow::Visible => "visible".into(),
            crate::prd::style::Overflow::Hidden => "hidden".into(),
            crate::prd::style::Overflow::Scroll => "scroll".into(),
        },
        "border-radius" | "borderRadius" => format!("{}px", style.border_radius.top_left),
        "box-shadow" | "boxShadow" => {
            if style.box_shadow.is_empty() {
                String::new()
            } else {
                let s = &style.box_shadow[0];
                let c = s.color.to_array();
                format!(
                    "{}px {}px {}px {}px rgba({},{},{},{}){}",
                    s.offset_x,
                    s.offset_y,
                    s.blur_radius,
                    s.spread_radius,
                    (c[0] * 255.0) as u8,
                    (c[1] * 255.0) as u8,
                    (c[2] * 255.0) as u8,
                    c[3],
                    if s.inset { " inset" } else { "" }
                )
            }
        }
        "color" => format!("rgba({},{},{},{})",
            (style.color.r * 255.0) as u8,
            (style.color.g * 255.0) as u8,
            (style.color.b * 255.0) as u8,
            style.color.a,
        ),
        "width" => match style.width {
            crate::prd::value::Dimension::Px(v) => format!("{}px", v),
            _ => "auto".into(),
        },
        "height" => match style.height {
            crate::prd::value::Dimension::Px(v) => format!("{}px", v),
            _ => "auto".into(),
        },
        _ => String::new(),
    }
}

fn style_dimension_px(dim: &crate::prd::value::Dimension, viewport: f32) -> Option<f32> {
    use crate::prd::value::Dimension;
    match dim {
        Dimension::Px(v) => Some(*v),
        Dimension::Percent(p) => Some(viewport * (*p / 100.0)),
        _ => None,
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// JavaScript DOM/Canvas2D Shim (identical to boa runtime)
// ═══════════════════════════════════════════════════════════════════════════

const JS_SHIM: &str = r#"
// OpenRender DOM/Canvas2D Shim
// Wraps native __or_* functions into standard web APIs

var __or_elementCache = {};
var __or_rafCallbacks = [];
var __or_rafId = 0;
var __or_canvasContexts = {};
var __or_startTime = __or_performance_now();
var __or_timeouts = [];
var __or_intervals = [];
var __or_nextTimerId = 1;

// ─── Helper: wrap a node ID into an Element-like object ───
function __or_wrapElement(nid) {
    if (nid < 0) return null;
    if (__or_elementCache[nid]) return __or_elementCache[nid];

    var el = {
        _nid: nid,
        _eventListeners: {},
        get id() { return __or_getNodeId(nid); },
        get tagName() { return __or_getNodeTag(nid); },
        get nodeName() { return __or_getNodeTag(nid); },
        get textContent() { return __or_getTextContent(nid); },
        set textContent(v) { __or_setTextContent(nid, String(v)); },
        get innerHTML() { return __or_getTextContent(nid); },
        set innerHTML(v) { __or_setInnerHTML(nid, String(v)); },
        get children() {
            var ids = JSON.parse(__or_getNodeChildren(nid));
            return ids.map(function(cid) { return __or_wrapElement(cid); });
        },
        getAttribute: function(name) {
            return __or_getNodeAttribute(nid, name);
        },
        setAttribute: function(name, value) {
            __or_setNodeAttribute(nid, name, String(value));
        },
        querySelector: function(sel) {
            var ids = JSON.parse(__or_querySelectorAll(sel));
            var children = JSON.parse(__or_getNodeChildren(nid));
            for (var i = 0; i < ids.length; i++) {
                if (isDescendant(nid, ids[i])) return __or_wrapElement(ids[i]);
            }
            return null;
        },
        querySelectorAll: function(sel) {
            var ids = JSON.parse(__or_querySelectorAll(sel));
            var result = [];
            for (var i = 0; i < ids.length; i++) {
                if (isDescendant(nid, ids[i])) result.push(__or_wrapElement(ids[i]));
            }
            return result;
        },
        addEventListener: function(type, fn) {
            if (!el._eventListeners[type]) el._eventListeners[type] = [];
            el._eventListeners[type].push(fn);
        },
        removeEventListener: function(type, fn) {
            if (!el._eventListeners[type]) return;
            var idx = el._eventListeners[type].indexOf(fn);
            if (idx >= 0) el._eventListeners[type].splice(idx, 1);
        },
        get classList() {
            return {
                add: function(c) { __or_classListOp(nid, 0, c); },
                remove: function(c) { __or_classListOp(nid, 1, c); },
                toggle: function(c, force) { return __or_classListOp(nid, 2, c, force); },
                contains: function(c) { return __or_classListOp(nid, 3, c); },
            };
        },
        get style() {
            return new Proxy({}, {
                get: function(t, p) {
                    if (p === 'setProperty') return function(name, val) {
                        if (name.startsWith('--')) {
                            __or_setRootStyleProperty(name, String(val));
                        } else {
                            __or_setStyle(nid, name, String(val));
                        }
                    };
                    if (p === 'getPropertyValue') return function(name) {
                        if (name.startsWith('--')) return __or_getComputedStyleVar(name);
                        return __or_getStyle(nid, name);
                    };
                    return __or_getStyle(nid, String(p));
                },
                set: function(t, p, v) {
                    __or_setStyle(nid, String(p), String(v));
                    return true;
                }
            });
        },
        getContext: function(type) {
            if (type !== '2d') return null;
            var cid = __or_getCanvasId(nid);
            if (cid < 0) return null;
            if (__or_canvasContexts[cid]) return __or_canvasContexts[cid];
            var ctx = __or_createContext2D(cid);
            __or_canvasContexts[cid] = ctx;
            return ctx;
        },
        get width() {
            var cid = __or_getCanvasId(nid);
            if (cid < 0) return 0;
            return parseInt(__or_canvasGetSize(cid).split(',')[0]);
        },
        set width(v) {
            var cid = __or_getCanvasId(nid);
            if (cid < 0) return;
            var h = parseInt(__or_canvasGetSize(cid).split(',')[1]);
            __or_canvasSetSize(cid, v, h);
        },
        get height() {
            var cid = __or_getCanvasId(nid);
            if (cid < 0) return 0;
            return parseInt(__or_canvasGetSize(cid).split(',')[1]);
        },
        set height(v) {
            var cid = __or_getCanvasId(nid);
            if (cid < 0) return;
            var w = parseInt(__or_canvasGetSize(cid).split(',')[0]);
            __or_canvasSetSize(cid, w, v);
        },
        getBoundingClientRect: function() {
            var rr = __or_getNodeRect(nid).split(',');
            var x = parseInt(rr[0]) || 0;
            var y = parseInt(rr[1]) || 0;
            var w = parseInt(rr[2]) || 0;
            var h = parseInt(rr[3]) || 0;
            return { x: x, y: y, width: w, height: h, top: y, left: x, bottom: y + h, right: x + w };
        },
        appendChild: function(child) {
            if (child && child._nid >= 0) __or_appendChild(nid, child._nid);
            return child;
        },
        removeChild: function(child) {
            if (child && child._nid >= 0) __or_removeChild(nid, child._nid);
            return child;
        },
        insertBefore: function(newChild, refChild) {
            var refNid = (refChild && refChild._nid >= 0) ? refChild._nid : -1;
            if (newChild && newChild._nid >= 0) __or_insertBefore(nid, newChild._nid, refNid);
            return newChild;
        },
        prepend: function() {
            for (var i = arguments.length - 1; i >= 0; i--) {
                var child = arguments[i];
                if (child && child._nid >= 0) __or_insertBefore(nid, child._nid, -1);
            }
        },
        remove: function() {
            var p = __or_getParentNode(nid);
            if (p >= 0) __or_removeChild(p, nid);
        },
        get parentElement() {
            var p = __or_getParentNode(nid);
            return p >= 0 ? __or_wrapElement(p) : null;
        },
        get parentNode() {
            var p = __or_getParentNode(nid);
            return p >= 0 ? __or_wrapElement(p) : null;
        },
        get firstChild() {
            var ids = JSON.parse(__or_getNodeChildren(nid));
            return ids.length > 0 ? __or_wrapElement(ids[0]) : null;
        },
        get lastChild() {
            var ids = JSON.parse(__or_getNodeChildren(nid));
            return ids.length > 0 ? __or_wrapElement(ids[ids.length - 1]) : null;
        },
        get childNodes() {
            var ids = JSON.parse(__or_getNodeChildren(nid));
            return ids.map(function(cid) { return __or_wrapElement(cid); });
        },
        get nextSibling() {
            var p = __or_getParentNode(nid);
            if (p < 0) return null;
            var siblings = JSON.parse(__or_getNodeChildren(p));
            var idx = siblings.indexOf(nid);
            if (idx >= 0 && idx < siblings.length - 1) return __or_wrapElement(siblings[idx + 1]);
            return null;
        },
        get previousSibling() {
            var p = __or_getParentNode(nid);
            if (p < 0) return null;
            var siblings = JSON.parse(__or_getNodeChildren(p));
            var idx = siblings.indexOf(nid);
            if (idx > 0) return __or_wrapElement(siblings[idx - 1]);
            return null;
        },
        get className() {
            return __or_getNodeAttribute(nid, 'class') || '';
        },
        set className(v) {
            __or_setNodeAttribute(nid, 'class', String(v));
        },
        set id(v) {
            __or_setNodeAttribute(nid, 'id', String(v));
        },
        get clientWidth() { return parseInt(__or_getNodeClientSize(nid).split(',')[0]) || 0; },
        get clientHeight() { return parseInt(__or_getNodeClientSize(nid).split(',')[1]) || 0; },
        get offsetWidth() { return parseInt(__or_getNodeClientSize(nid).split(',')[0]) || 0; },
        get offsetHeight() { return parseInt(__or_getNodeClientSize(nid).split(',')[1]) || 0; },
        get offsetLeft() { return parseInt(__or_getNodeRect(nid).split(',')[0]) || 0; },
        get offsetTop() { return parseInt(__or_getNodeRect(nid).split(',')[1]) || 0; },
    };
    __or_elementCache[nid] = el;
    return el;
}

function isDescendant(parentNid, childNid) {
    var cur = childNid;
    while (cur >= 0) {
        if (cur === parentNid) return true;
        cur = __or_getParentNode(cur);
    }
    return false;
}

// ─── Canvas 2D Context Factory ───
function __or_createContext2D(cid) {
    var _fillStyleStr = '#000000';
    var _strokeStyleStr = '#000000';
    var _lineWidth = 1;
    var _globalAlpha = 1;
    var _globalCompositeOperation = 'source-over';
    var _font = '10px sans-serif';
    var _textAlign = 'start';
    var _textBaseline = 'alphabetic';
    var _lineCap = 'butt';
    var _lineJoin = 'miter';
    var _shadowBlur = 0;
    var _shadowColor = 'rgba(0, 0, 0, 0)';
    var _imageSmoothingEnabled = true;
    var _miterLimit = 10;
    var _lineDashOffset = 0;

    var ctx = {
        get canvas() {
            for (var nid in __or_elementCache) {
                if (__or_getCanvasId(parseInt(nid)) === cid) return __or_elementCache[nid];
            }
            return null;
        },
        get fillStyle() { return _fillStyleStr; },
        set fillStyle(v) {
            _fillStyleStr = v;
            if (typeof v === 'string') {
                __or_c2d_setFillStyle(cid, v);
            } else if (v && v._type === 'gradient') {
                __or_c2d_setFillGradient(cid, v._id);
            } else if (v && v._type === 'pattern') {
                __or_c2d_setFillPattern(cid, v._id);
            }
        },
        get strokeStyle() { return _strokeStyleStr; },
        set strokeStyle(v) {
            _strokeStyleStr = v;
            if (typeof v === 'string') {
                __or_c2d_setStrokeStyle(cid, v);
            } else if (v && v._type === 'gradient') {
                __or_c2d_setStrokeGradient(cid, v._id);
            }
        },
        get lineWidth() { return _lineWidth; },
        set lineWidth(v) { _lineWidth = v; __or_c2d(cid, 16, v, 0, 0, 0, 0, 0); },
        get globalAlpha() { return _globalAlpha; },
        set globalAlpha(v) { _globalAlpha = v; __or_c2d(cid, 17, v, 0, 0, 0, 0, 0); },
        get globalCompositeOperation() { return _globalCompositeOperation; },
        set globalCompositeOperation(v) { _globalCompositeOperation = v; __or_c2d_setBlendMode(cid, v); },
        get font() { return _font; },
        set font(v) { _font = v; __or_c2d_setFont(cid, v); },
        get textAlign() { return _textAlign; },
        set textAlign(v) { _textAlign = v; __or_c2d_setTextAlign(cid, v); },
        get textBaseline() { return _textBaseline; },
        set textBaseline(v) { _textBaseline = v; __or_c2d_setTextBaseline(cid, v); },
        get lineCap() { return _lineCap; },
        set lineCap(v) { _lineCap = v; __or_c2d_setLineCap(cid, v); },
        get lineJoin() { return _lineJoin; },
        set lineJoin(v) { _lineJoin = v; __or_c2d_setLineJoin(cid, v); },
        get shadowBlur() { return _shadowBlur; },
        set shadowBlur(v) { _shadowBlur = Number(v) || 0; },
        get shadowColor() { return _shadowColor; },
        set shadowColor(v) { _shadowColor = String(v); },
        get imageSmoothingEnabled() { return _imageSmoothingEnabled; },
        set imageSmoothingEnabled(v) { _imageSmoothingEnabled = !!v; },
        get miterLimit() { return _miterLimit; },
        set miterLimit(v) { _miterLimit = Number(v) || 10; __or_c2d_setMiterLimit(cid, _miterLimit); },
        get lineDashOffset() { return _lineDashOffset; },
        set lineDashOffset(v) { _lineDashOffset = Number(v) || 0; },

        fillRect: function(x, y, w, h) {
            if (_fillStyleStr && typeof _fillStyleStr === 'object') {
                if (_fillStyleStr._type === 'pattern') {
                    __or_c2d_fillRectPattern(cid, x, y, w, h);
                } else {
                    __or_c2d_fillRectGrad(cid, x, y, w, h);
                }
            } else {
                __or_c2d(cid, 1, x, y, w, h, 0, 0);
            }
        },
        strokeRect: function(x, y, w, h) { __or_c2d(cid, 2, x, y, w, h, 0, 0); },
        clearRect: function(x, y, w, h) { __or_c2d(cid, 3, x, y, w, h, 0, 0); },
        beginPath: function() { __or_c2d(cid, 4, 0, 0, 0, 0, 0, 0); },
        closePath: function() { __or_c2d(cid, 5, 0, 0, 0, 0, 0, 0); },
        moveTo: function(x, y) { __or_c2d(cid, 6, x, y, 0, 0, 0, 0); },
        lineTo: function(x, y) { __or_c2d(cid, 7, x, y, 0, 0, 0, 0); },
        arc: function(x, y, r, start, end, ccw) { __or_c2d(cid, 8, x, y, r, start, end, ccw ? 1 : 0); },
        fill: function() { __or_c2d(cid, 9, 0, 0, 0, 0, 0, 0); },
        stroke: function() { __or_c2d(cid, 10, 0, 0, 0, 0, 0, 0); },
        save: function() { __or_c2d(cid, 11, 0, 0, 0, 0, 0, 0); },
        restore: function() { __or_c2d(cid, 12, 0, 0, 0, 0, 0, 0); },
        translate: function(x, y) { __or_c2d(cid, 13, x, y, 0, 0, 0, 0); },
        rotate: function(a) { __or_c2d(cid, 14, a, 0, 0, 0, 0, 0); },
        scale: function(x, y) { __or_c2d(cid, 15, x, y, 0, 0, 0, 0); },
        setTransform: function(a, b, c, d, e, f) { __or_c2d(cid, 20, a, b, c, d, e, f); },
        resetTransform: function() { __or_c2d(cid, 21, 0, 0, 0, 0, 0, 0); },
        bezierCurveTo: function(cp1x, cp1y, cp2x, cp2y, x, y) { __or_c2d(cid, 18, cp1x, cp1y, cp2x, cp2y, x, y); },
        quadraticCurveTo: function(cpx, cpy, x, y) { __or_c2d(cid, 19, cpx, cpy, x, y, 0, 0); },

        drawImage: function(source) {
            if (!source || !source._nid) return;
            var srcCid = __or_getCanvasId(source._nid);
            if (srcCid < 0) return;
            if (arguments.length >= 5) {
                __or_c2d_drawImage(cid, srcCid, arguments[1], arguments[2], arguments[3], arguments[4]);
            } else if (arguments.length >= 3) {
                var size = __or_canvasGetSize(srcCid).split(',');
                __or_c2d_drawImage(cid, srcCid, arguments[1], arguments[2], parseInt(size[0]), parseInt(size[1]));
            }
        },

        createRadialGradient: function(x0, y0, r0, x1, y1, r1) {
            var gid = __or_c2d_createRadialGradient(x0, y0, r0, x1, y1, r1);
            return {
                _type: 'gradient',
                _id: gid,
                addColorStop: function(offset, color) {
                    __or_c2d_gradientAddStop(gid, offset, color);
                }
            };
        },
        createLinearGradient: function(x0, y0, x1, y1) {
            var gid = __or_c2d_createLinearGradient(x0, y0, x1, y1);
            return {
                _type: 'gradient',
                _id: gid,
                addColorStop: function(offset, color) {
                    __or_c2d_gradientAddStop(gid, offset, color);
                }
            };
        },
        createPattern: function(source, repeat) {
            if (!source || !source._nid) return null;
            var srcCid = __or_getCanvasId(source._nid);
            if (srcCid < 0) return null;
            return {
                _type: 'pattern',
                _id: srcCid,
            };
        },
        fillText: function(text, x, y) { __or_c2d_fillText(cid, text, x, y); },
        strokeText: function() {},
        measureText: function(text) {
            var sz = parseFloat(_font) || 10;
            return { width: text.length * sz * 0.6 };
        },
        getImageData: function(x, y, w, h) {
            var raw = __or_c2d_getImageData(cid, x|0, y|0, w|0, h|0);
            var parsed = { width: 0, height: 0, data: [] };
            try { parsed = JSON.parse(raw); } catch(_) {}
            return {
                width: parsed.width || 0,
                height: parsed.height || 0,
                data: parsed.data || []
            };
        },
        putImageData: function(imageData, x, y) {
            if (!imageData || !imageData.data) return;
            __or_c2d_putImageData(
                cid,
                x|0,
                y|0,
                (imageData.width || 0)|0,
                (imageData.height || 0)|0,
                JSON.stringify(Array.prototype.slice.call(imageData.data))
            );
        },
        clip: function() { __or_c2d_clipPath(cid); },
        setLineDash: function(segments) {},
        getLineDash: function() { return []; },
    };
    return ctx;
}

// ─── document object ───
var document = {
    getElementById: function(id) {
        var nid = __or_getElementById(id);
        return __or_wrapElement(nid);
    },
    querySelector: function(sel) {
        var ids = JSON.parse(__or_querySelectorAll(sel));
        if (ids.length > 0) return __or_wrapElement(ids[0]);
        return null;
    },
    querySelectorAll: function(sel) {
        var ids = JSON.parse(__or_querySelectorAll(sel));
        return ids.map(function(id) { return __or_wrapElement(id); });
    },
    createElement: function(tag) {
        var nid = __or_createElement(tag);
        return __or_wrapElement(nid);
    },
    get documentElement() {
        return __or_wrapElement(0);
    },
    get body() {
        return __or_wrapElement(0);
    },
};

// ─── window object ───
var window = (typeof globalThis !== 'undefined') ? globalThis : {};
window.document = document;
var __or_viewport = __or_getViewportSize().split(',');
window.innerWidth = parseInt(__or_viewport[0]) || 1920;
window.innerHeight = parseInt(__or_viewport[1]) || 1080;
window.top = window;
window.self = window;
window.parent = window;

var __or_globalListeners = {};
function addEventListener(type, fn) {
    if (!__or_globalListeners[type]) __or_globalListeners[type] = [];
    __or_globalListeners[type].push(fn);
}
function removeEventListener(type, fn) {
    if (!__or_globalListeners[type]) return;
    var idx = __or_globalListeners[type].indexOf(fn);
    if (idx >= 0) __or_globalListeners[type].splice(idx, 1);
}
window.addEventListener = addEventListener;
window.removeEventListener = removeEventListener;

// ─── console ───
var console = {
    log: function() { __or_log(1, Array.prototype.slice.call(arguments).join(' ')); },
    warn: function() { __or_log(2, Array.prototype.slice.call(arguments).join(' ')); },
    error: function() { __or_log(3, Array.prototype.slice.call(arguments).join(' ')); },
    info: function() { __or_log(1, Array.prototype.slice.call(arguments).join(' ')); },
    debug: function() { __or_log(0, Array.prototype.slice.call(arguments).join(' ')); },
    dumpDoc: function() { __or_dumpDoc(); },
};

// ─── performance ───
var performance = {
    now: function() { return __or_performance_now() - __or_startTime; },
};

// ─── requestAnimationFrame ───
function requestAnimationFrame(cb) {
    var id = ++__or_rafId;
    __or_rafCallbacks.push({ id: id, callback: cb });
    return id;
}
function cancelAnimationFrame(id) {
    __or_rafCallbacks = __or_rafCallbacks.filter(function(c) { return c.id !== id; });
}

// ─── setTimeout / setInterval ───
function setTimeout(fn, delay) {
    var id = __or_nextTimerId++;
    var triggerAt = __or_performance_now() + (delay || 0);
    __or_timeouts.push({ id: id, callback: fn, triggerAt: triggerAt });
    return id;
}
function clearTimeout(id) {
    __or_timeouts = __or_timeouts.filter(function(t) { return t.id !== id; });
}
function setInterval(fn, delay) {
    var id = __or_nextTimerId++;
    var interval = delay || 16;
    var triggerAt = __or_performance_now() + interval;
    __or_intervals.push({ id: id, callback: fn, interval: interval, triggerAt: triggerAt });
    return id;
}
function clearInterval(id) {
    __or_intervals = __or_intervals.filter(function(t) { return t.id !== id; });
}

// ─── getComputedStyle ───
function getComputedStyle(el) {
    return {
        getPropertyValue: function(name) {
            if (name.startsWith('--')) return __or_getComputedStyleVar(name);
            if (el && el._nid !== undefined) return __or_getStyle(el._nid, name);
            return '';
        }
    };
}

// ─── rAF tick function (called by Rust each frame) ───
function __or_raf_tick(timestamp) {
    var now = __or_performance_now();
    var pendingTimeouts = [];
    var remaining = [];
    for (var i = 0; i < __or_timeouts.length; i++) {
        if (now >= __or_timeouts[i].triggerAt) {
            pendingTimeouts.push(__or_timeouts[i]);
        } else {
            remaining.push(__or_timeouts[i]);
        }
    }
    __or_timeouts = remaining;
    for (var j = 0; j < pendingTimeouts.length; j++) {
        try { pendingTimeouts[j].callback(); } catch(e) { console.error('Timeout error:', e); }
    }

    for (var k = 0; k < __or_intervals.length; k++) {
        if (now >= __or_intervals[k].triggerAt) {
            try { __or_intervals[k].callback(); } catch(e) { console.error('Interval error:', e); }
            __or_intervals[k].triggerAt = now + __or_intervals[k].interval;
        }
    }

    var callbacks = __or_rafCallbacks.slice();
    __or_rafCallbacks = [];
    for (var m = 0; m < callbacks.length; m++) {
        var entry = callbacks[m];
        if (typeof entry.callback !== 'function') {
            console.error('[PRISM] rAF entry not callable: type=' + typeof entry.callback + ' id=' + entry.id);
            continue;
        }
        try { entry.callback(timestamp); } catch(e) { console.error('rAF error:', e); }
    }
}

if (typeof Math.clamp === 'undefined') {
    Math.clamp = function(v, min, max) { return Math.min(Math.max(v, min), max); };
}

if (typeof Array.from === 'undefined') {
    Array.from = function(arr) {
        var result = [];
        for (var i = 0; i < arr.length; i++) result.push(arr[i]);
        return result;
    };
}

try {
    var __vp = __or_getViewportSize().split(',');
    window.innerWidth = parseInt(__vp[0]) || 1920;
    window.innerHeight = parseInt(__vp[1]) || 1080;
} catch(e) {}

// ─── DOM event dispatch (called from Rust) ───
// Walk from the target element up through ancestors (bubble phase).
function __or_dispatchDomEvent(nodeId, eventType) {
    var nid = nodeId;
    while (nid >= 0) {
        var el = __or_elementCache[nid];
        if (el && el._eventListeners && el._eventListeners[eventType]) {
            var fns = el._eventListeners[eventType].slice();
            var evt = { type: eventType, target: __or_wrapElement(nodeId), currentTarget: el, stopPropagation: function(){nid=-1;}, preventDefault: function(){} };
            for (var i = 0; i < fns.length; i++) {
                try { fns[i](evt); } catch(e) { console.error(eventType + ' handler error:', e); }
            }
            if (nid < 0) break; // stopPropagation was called
        }
        // Walk up to parent.
        var parentStr = __or_getParentNode(nid);
        nid = (typeof parentStr === 'number') ? parentStr : parseInt(parentStr);
        if (isNaN(nid)) break;
    }
}
"#;

