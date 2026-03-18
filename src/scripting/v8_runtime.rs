// canvasx-runtime/src/scripting/v8_runtime.rs
//
// JavaScript runtime powered by V8 (via the `v8` crate / rusty_v8).
// Drop-in replacement for the boa_engine-based runtime — same public API,
// but backed by Google's V8 JIT engine for 100–1000× faster JS execution.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::time::Instant;

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeKind, NodeId};
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

fn reachable_nodes(doc: &CxrdDocument) -> Vec<bool> {
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

/// Shared mutable state accessible from both Rust and JS native functions.
pub struct SharedState {
    pub document: CxrdDocument,
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
        document: CxrdDocument,
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
                    Some(_) => log::info!("[CX][JS] V8 shim injected OK ({} bytes)", JS_SHIM.len()),
                    None => {
                        let msg = tc.exception()
                            .map(|e| e.to_rust_string_lossy(&tc))
                            .unwrap_or_default();
                        log::error!("[CX][JS] V8 shim execution FAILED: {}", msg);
                    }
                },
                None => {
                    let msg = tc.exception()
                        .map(|e| e.to_rust_string_lossy(&tc))
                        .unwrap_or_default();
                    log::error!("[CX][JS] V8 shim compilation FAILED: {}", msg);
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
        log::warn!("[CX][JS] Executing script '{}' ({} bytes)", name, source.len());

        let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
        let mut hs = hs.init();
        let ctx = v8::Local::new(&hs, &self.context);
        let cs = &mut v8::ContextScope::new(&mut hs, ctx);

        let code = match v8::String::new(cs, source) {
            Some(s) => s,
            None => {
                log::error!("[CX][JS] Failed to create V8 string for '{}'", name);
                return;
            }
        };

        let tc = std::pin::pin!(v8::TryCatch::new(cs));
        let tc = tc.init();
        match v8::Script::compile(&tc, code, None) {
            Some(script) => match script.run(&tc) {
                Some(_) => {
                    log::debug!("[CX][JS] Script '{}' completed OK", name);
                    let state = self.state.borrow();
                    log::warn!(
                        "[CX][JS] Doc state: {} nodes, {} canvases, layout_dirty={}",
                        state.document.nodes.len(),
                        state.node_canvas_map.len(),
                        state.layout_dirty,
                    );
                }
                None => {
                    let msg = tc.exception()
                        .map(|e| e.to_rust_string_lossy(&tc))
                        .unwrap_or_default();
                    log::error!("[CX][JS] Script '{}' THREW: {}", name, msg);
                }
            },
            None => {
                let msg = tc.exception()
                    .map(|e| e.to_rust_string_lossy(&tc))
                    .unwrap_or_default();
                log::error!("[CX][JS] Script '{}' compile error: {}", name, msg);
            }
        }
    }

    /// Execute a script file from disk.
    pub fn execute_file(&mut self, path: &Path) {
        self.activate();
        log::warn!("[CX][JS] Loading script file: {}", path.display());
        match std::fs::read_to_string(path) {
            Ok(source) => {
                let name = path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("unknown");
                self.execute(&source, name);
            }
            Err(e) => log::error!("[CX][JS] Failed to read script '{}': {}", path.display(), e),
        }
    }

    /// Resolve and cache the global __cx_raf_tick function.
    pub fn cache_raf_tick_fn(&mut self) {
        let global_fn = {
            let hs = std::pin::pin!(v8::HandleScope::new(&mut self.isolate));
            let mut hs = hs.init();
            let ctx = v8::Local::new(&hs, &self.context);
            let cs = &mut v8::ContextScope::new(&mut hs, ctx);

            let global = ctx.global(cs);
            let key = v8::String::new(cs, "__cx_raf_tick").unwrap();
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
            log::info!("[CX][JS] Cached __cx_raf_tick function for direct calls");
        } else {
            log::warn!("[CX][JS] __cx_raf_tick not found or not callable — will fall back to eval");
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
                        log::error!("[CX][JS] tick error: {}", msg);
                    }
                }
            } else {
                let code_str = format!(
                    "if(typeof __cx_raf_tick==='function')__cx_raf_tick({});",
                    now_ms
                );
                if let Some(code) = v8::String::new(cs, &code_str) {
                    let tc = std::pin::pin!(v8::TryCatch::new(cs));
                    let tc = tc.init();
                    if let Some(script) = v8::Script::compile(&tc, code, None) {
                        if script.run(&tc).is_none() {
                            if let Some(ex) = tc.exception() {
                                let msg = ex.to_rust_string_lossy(&tc);
                                log::error!("[CX][JS] tick eval error: {}", msg);
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

    /// Get the document (for layout/paint passes).
    pub fn document(&self) -> std::cell::Ref<'_, CxrdDocument> {
        std::cell::Ref::map(self.state.borrow(), |s| &s.document)
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
        log::debug!("[CX][JS] Restyled document with {} rules", rules.len());
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
    set_fn!("__cx_log", cx_log);
    // Performance
    set_fn!("__cx_performance_now", cx_performance_now);
    // DOM
    set_fn!("__cx_getElementById", cx_get_element_by_id);
    set_fn!("__cx_querySelector", cx_query_selector);
    set_fn!("__cx_querySelectorAll", cx_query_selector_all);
    set_fn!("__cx_createElement", cx_create_element);
    set_fn!("__cx_getTextContent", cx_get_text_content);
    set_fn!("__cx_setTextContent", cx_set_text_content);
    set_fn!("__cx_setInnerHTML", cx_set_inner_html);
    set_fn!("__cx_getNodeAttribute", cx_get_node_attribute);
    set_fn!("__cx_setNodeAttribute", cx_set_node_attribute);
    set_fn!("__cx_classListOp", cx_class_list_op);
    set_fn!("__cx_getStyle", cx_get_style);
    set_fn!("__cx_setStyle", cx_set_style);
    set_fn!("__cx_getComputedStyleVar", cx_get_computed_style_var);
    set_fn!("__cx_setRootStyleProperty", cx_set_root_style_property);
    set_fn!("__cx_getNodeTag", cx_get_node_tag);
    set_fn!("__cx_getNodeChildren", cx_get_node_children);
    set_fn!("__cx_getNodeId", cx_get_node_id);
    set_fn!("__cx_appendChild", cx_append_child);
    set_fn!("__cx_removeChild", cx_remove_child);
    set_fn!("__cx_getParentNode", cx_get_parent_node);
    set_fn!("__cx_insertBefore", cx_insert_before);
    set_fn!("__cx_getNodeClientSize", cx_get_node_client_size);
    set_fn!("__cx_getNodeRect", cx_get_node_rect);
    // Canvas 2D
    set_fn!("__cx_getCanvasId", cx_get_canvas_id);
    set_fn!("__cx_canvasSetSize", cx_canvas_set_size);
    set_fn!("__cx_canvasGetSize", cx_canvas_get_size);
    set_fn!("__cx_c2d", cx_c2d);
    set_fn!("__cx_c2d_setFillStyle", cx_c2d_set_fill_style);
    set_fn!("__cx_c2d_setStrokeStyle", cx_c2d_set_stroke_style);
    set_fn!("__cx_c2d_setFillGradient", cx_c2d_set_fill_gradient);
    set_fn!("__cx_c2d_setStrokeGradient", cx_c2d_set_stroke_gradient);
    set_fn!("__cx_c2d_setFillPattern", cx_c2d_set_fill_pattern);
    set_fn!("__cx_c2d_setBlendMode", cx_c2d_set_blend_mode);
    set_fn!("__cx_c2d_setFont", cx_c2d_set_font);
    set_fn!("__cx_c2d_setTextAlign", cx_c2d_set_text_align);
    set_fn!("__cx_c2d_setTextBaseline", cx_c2d_set_text_baseline);
    set_fn!("__cx_c2d_fillText", cx_c2d_fill_text);
    set_fn!("__cx_c2d_drawImage", cx_c2d_draw_image);
    set_fn!("__cx_c2d_createRadialGradient", cx_c2d_create_radial_gradient);
    set_fn!("__cx_c2d_createLinearGradient", cx_c2d_create_linear_gradient);
    set_fn!("__cx_c2d_gradientAddStop", cx_c2d_gradient_add_stop);
    set_fn!("__cx_c2d_createPattern", cx_c2d_create_pattern);
    set_fn!("__cx_c2d_fillRectGrad", cx_c2d_fill_rect_grad);
    set_fn!("__cx_c2d_fillRectPattern", cx_c2d_fill_rect_pattern);
    set_fn!("__cx_c2d_getImageData", cx_c2d_get_image_data);
    set_fn!("__cx_c2d_putImageData", cx_c2d_put_image_data);
    set_fn!("__cx_c2d_clipPath", cx_c2d_clip_path);
    // IPC
    set_fn!("__cx_ipc_send", cx_ipc_send);
    // Misc
    set_fn!("__cx_setDataValue", cx_set_data_value);
    set_fn!("__cx_getViewportSize", cx_get_viewport_size);
    set_fn!("__cx_dumpDoc", cx_dump_doc);
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
}

// ═══════════════════════════════════════════════════════════════════════════
// Performance
// ═══════════════════════════════════════════════════════════════════════════

fn cx_performance_now(scope: &mut v8::PinScope<'_, '_>, _args: v8::FunctionCallbackArguments, mut rv: v8::ReturnValue<v8::Value>) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64() * 1000.0;
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
        let mut node = crate::cxrd::node::CxrdNode::container(0);
        node.tag = Some(tag.clone());
        if tag == "canvas" {
            node.kind = crate::cxrd::node::NodeKind::Canvas { width: 300, height: 150 };
        }
        let node_id = st.document.add_node(node);
        if tag == "canvas" {
            let cid = st.canvas_manager.create_canvas(300, 150);
            st.node_canvas_map.insert(node_id, cid);
            st.canvas_node_map.insert(cid, node_id);
            log::info!("[CX][DOM] createElement('canvas') → node={} canvas={}", node_id, cid);
        } else {
            log::info!("[CX][DOM] createElement('{}') → node={}", tag, node_id);
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

        // Clear existing children.
        if let Some(node) = st.document.get_node_mut(nid) {
            node.children.clear();
        }

        // Create a new text child that inherits the parent's inheritable styles.
        let mut text_node = crate::cxrd::node::CxrdNode::text(0, text);
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
    with_state(|st| {
        log::info!("[CX][DOM] setInnerHTML: node={} html_len={}", nid, html.len());
        set_inner_html(st, nid, &html);
    });
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
        if let Some(node) = st.document.get_node_mut(nid) {
            match name.as_str() {
                "id" => {
                    node.html_id = if value.is_empty() { None } else { Some(value.clone()) };
                    node.attributes.insert(name, value);
                }
                "class" => {
                    node.classes = value.split_whitespace().map(String::from).collect();
                    node.attributes.insert(name, value);
                }
                _ => {
                    node.attributes.insert(name, value);
                }
            }
            st.layout_dirty = true;
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
        if let Some(node) = st.document.get_node_mut(nid) {
            match op {
                0 => { // add
                    if !node.classes.contains(&class_name) {
                        node.classes.push(class_name.clone());
                        st.layout_dirty = true;
                    }
                    true
                }
                1 => { // remove
                    if let Some(pos) = node.classes.iter().position(|c| c == &class_name) {
                        node.classes.remove(pos);
                        st.layout_dirty = true;
                    }
                    false
                }
                2 => { // toggle
                    let has = node.classes.contains(&class_name);
                    let should_add = force.unwrap_or(!has);
                    if should_add && !has {
                        node.classes.push(class_name.clone());
                        st.layout_dirty = true;
                        true
                    } else if !should_add && has {
                        if let Some(pos) = node.classes.iter().position(|c| c == &class_name) {
                            node.classes.remove(pos);
                            st.layout_dirty = true;
                        }
                        false
                    } else {
                        has
                    }
                }
                3 => node.classes.contains(&class_name), // contains
                _ => false,
            }
        } else {
            false
        }
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
        log::warn!("[CX][DEBUG] === Document dump: {} nodes ===", st.document.nodes.len());
        for node in &st.document.nodes {
            let tag = node.tag.as_deref().unwrap_or("(none)");
            let id = node.html_id.as_deref().unwrap_or("");
            let classes = node.classes.join(" ");
            let kind = match &node.kind {
                NodeKind::Container => "Container",
                NodeKind::Text { content } => {
                    log::warn!("[CX][DEBUG]   node {} tag={} id={} class='{}' kind=Text text='{}'",
                        node.id, tag, id, classes, &content[..content.len().min(60)]);
                    continue;
                },
                NodeKind::Canvas { .. } => "Canvas",
                _ => "Other",
            };
            let children_str: Vec<String> = node.children.iter().map(|c| c.to_string()).collect();
            let has_canvas = st.node_canvas_map.contains_key(&node.id);
            log::warn!("[CX][DEBUG]   node {} tag={} id={} class='{}' kind={} children=[{}] canvas={}",
                node.id, tag, id, classes, kind, children_str.join(","), has_canvas);
        }
        log::warn!("[CX][DEBUG] === End document dump ===");
    });
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper functions (engine-agnostic)
// ═══════════════════════════════════════════════════════════════════════════

fn selector_matches_node(selector: &str, node: &crate::cxrd::node::CxrdNode, _doc: &CxrdDocument) -> bool {
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

fn collect_text_content(doc: &CxrdDocument, node_id: NodeId) -> String {
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
            *node = crate::cxrd::node::CxrdNode::container(child_id);
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

fn add_html_children(st: &mut SharedState, parent_id: NodeId, html: &str) {
    let bytes = html.as_bytes();
    let mut pos = 0usize;

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

            let mut node = crate::cxrd::node::CxrdNode::container(0);
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
                    for (prop, val) in &rule.declarations {
                        apply_property(&mut node.style, prop, val, &st.css_variables);
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
            let text = html[text_start..pos].trim();
            if text.is_empty() { continue; }

            let parent = *node_stack.last().unwrap_or(&parent_id);
            let mut text_node = crate::cxrd::node::CxrdNode::text(0, text);
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
}

fn collect_ancestor_chain(doc: &CxrdDocument, node_id: NodeId) -> Vec<AncestorInfo> {
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

fn apply_dynamic_tag_defaults(node: &mut crate::cxrd::node::CxrdNode) {
    use crate::cxrd::style::{Display, FlexDirection, FontWeight};

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

fn simple_rule_matches(selector: &str, node: &crate::cxrd::node::CxrdNode) -> bool {
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

fn get_computed_style_value(style: &crate::cxrd::style::ComputedStyle, prop: &str) -> String {
    use crate::cxrd::style::Display;
    match prop {
        "display" => match style.display {
            Display::None => "none".into(),
            Display::Block => "block".into(),
            Display::Flex => "flex".into(),
            Display::InlineBlock => "inline-block".into(),
            Display::Grid => "grid".into(),
        },
        "opacity" => format!("{}", style.opacity),
        "background" | "background-color" | "backgroundColor" => {
            match &style.background {
                crate::cxrd::style::Background::Solid(c) => {
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
            use crate::cxrd::value::Dimension;
            match style.padding.top {
                Dimension::Px(v) => format!("{}px", v),
                Dimension::Percent(v) => format!("{}%", v),
                _ => String::new(),
            }
        },
        "overflow" => "visible".into(),
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
            crate::cxrd::value::Dimension::Px(v) => format!("{}px", v),
            _ => "auto".into(),
        },
        "height" => match style.height {
            crate::cxrd::value::Dimension::Px(v) => format!("{}px", v),
            _ => "auto".into(),
        },
        _ => String::new(),
    }
}

fn style_dimension_px(dim: &crate::cxrd::value::Dimension, viewport: f32) -> Option<f32> {
    use crate::cxrd::value::Dimension;
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
// CanvasX DOM/Canvas2D Shim
// Wraps native __cx_* functions into standard web APIs

var __cx_elementCache = {};
var __cx_rafCallbacks = [];
var __cx_rafId = 0;
var __cx_canvasContexts = {};
var __cx_startTime = __cx_performance_now();
var __cx_timeouts = [];
var __cx_intervals = [];
var __cx_nextTimerId = 1;

// ─── Helper: wrap a node ID into an Element-like object ───
function __cx_wrapElement(nid) {
    if (nid < 0) return null;
    if (__cx_elementCache[nid]) return __cx_elementCache[nid];

    var el = {
        _nid: nid,
        _eventListeners: {},
        get id() { return __cx_getNodeId(nid); },
        get tagName() { return __cx_getNodeTag(nid); },
        get nodeName() { return __cx_getNodeTag(nid); },
        get textContent() { return __cx_getTextContent(nid); },
        set textContent(v) { __cx_setTextContent(nid, String(v)); },
        get innerHTML() { return ''; },
        set innerHTML(v) { __cx_setInnerHTML(nid, String(v)); },
        get children() {
            var ids = JSON.parse(__cx_getNodeChildren(nid));
            return ids.map(function(cid) { return __cx_wrapElement(cid); });
        },
        getAttribute: function(name) {
            return __cx_getNodeAttribute(nid, name);
        },
        setAttribute: function(name, value) {
            __cx_setNodeAttribute(nid, name, String(value));
        },
        querySelector: function(sel) {
            var ids = JSON.parse(__cx_querySelectorAll(sel));
            var children = JSON.parse(__cx_getNodeChildren(nid));
            for (var i = 0; i < ids.length; i++) {
                if (isDescendant(nid, ids[i])) return __cx_wrapElement(ids[i]);
            }
            return null;
        },
        querySelectorAll: function(sel) {
            var ids = JSON.parse(__cx_querySelectorAll(sel));
            var result = [];
            for (var i = 0; i < ids.length; i++) {
                if (isDescendant(nid, ids[i])) result.push(__cx_wrapElement(ids[i]));
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
                add: function(c) { __cx_classListOp(nid, 0, c); },
                remove: function(c) { __cx_classListOp(nid, 1, c); },
                toggle: function(c, force) { return __cx_classListOp(nid, 2, c, force); },
                contains: function(c) { return __cx_classListOp(nid, 3, c); },
            };
        },
        get style() {
            return new Proxy({}, {
                get: function(t, p) {
                    if (p === 'setProperty') return function(name, val) {
                        if (name.startsWith('--')) {
                            __cx_setRootStyleProperty(name, String(val));
                        } else {
                            __cx_setStyle(nid, name, String(val));
                        }
                    };
                    if (p === 'getPropertyValue') return function(name) {
                        if (name.startsWith('--')) return __cx_getComputedStyleVar(name);
                        return __cx_getStyle(nid, name);
                    };
                    return __cx_getStyle(nid, String(p));
                },
                set: function(t, p, v) {
                    __cx_setStyle(nid, String(p), String(v));
                    return true;
                }
            });
        },
        getContext: function(type) {
            if (type !== '2d') return null;
            var cid = __cx_getCanvasId(nid);
            if (cid < 0) return null;
            if (__cx_canvasContexts[cid]) return __cx_canvasContexts[cid];
            var ctx = __cx_createContext2D(cid);
            __cx_canvasContexts[cid] = ctx;
            return ctx;
        },
        get width() {
            var cid = __cx_getCanvasId(nid);
            if (cid < 0) return 0;
            return parseInt(__cx_canvasGetSize(cid).split(',')[0]);
        },
        set width(v) {
            var cid = __cx_getCanvasId(nid);
            if (cid < 0) return;
            var h = parseInt(__cx_canvasGetSize(cid).split(',')[1]);
            __cx_canvasSetSize(cid, v, h);
        },
        get height() {
            var cid = __cx_getCanvasId(nid);
            if (cid < 0) return 0;
            return parseInt(__cx_canvasGetSize(cid).split(',')[1]);
        },
        set height(v) {
            var cid = __cx_getCanvasId(nid);
            if (cid < 0) return;
            var w = parseInt(__cx_canvasGetSize(cid).split(',')[0]);
            __cx_canvasSetSize(cid, w, v);
        },
        getBoundingClientRect: function() {
            var rr = __cx_getNodeRect(nid).split(',');
            var x = parseInt(rr[0]) || 0;
            var y = parseInt(rr[1]) || 0;
            var w = parseInt(rr[2]) || 0;
            var h = parseInt(rr[3]) || 0;
            return { x: x, y: y, width: w, height: h, top: y, left: x, bottom: y + h, right: x + w };
        },
        appendChild: function(child) {
            if (child && child._nid >= 0) __cx_appendChild(nid, child._nid);
            return child;
        },
        removeChild: function(child) {
            if (child && child._nid >= 0) __cx_removeChild(nid, child._nid);
            return child;
        },
        insertBefore: function(newChild, refChild) {
            var refNid = (refChild && refChild._nid >= 0) ? refChild._nid : -1;
            if (newChild && newChild._nid >= 0) __cx_insertBefore(nid, newChild._nid, refNid);
            return newChild;
        },
        prepend: function() {
            for (var i = arguments.length - 1; i >= 0; i--) {
                var child = arguments[i];
                if (child && child._nid >= 0) __cx_insertBefore(nid, child._nid, -1);
            }
        },
        remove: function() {
            var p = __cx_getParentNode(nid);
            if (p >= 0) __cx_removeChild(p, nid);
        },
        get parentElement() {
            var p = __cx_getParentNode(nid);
            return p >= 0 ? __cx_wrapElement(p) : null;
        },
        get parentNode() {
            var p = __cx_getParentNode(nid);
            return p >= 0 ? __cx_wrapElement(p) : null;
        },
        get firstChild() {
            var ids = JSON.parse(__cx_getNodeChildren(nid));
            return ids.length > 0 ? __cx_wrapElement(ids[0]) : null;
        },
        get lastChild() {
            var ids = JSON.parse(__cx_getNodeChildren(nid));
            return ids.length > 0 ? __cx_wrapElement(ids[ids.length - 1]) : null;
        },
        get childNodes() {
            var ids = JSON.parse(__cx_getNodeChildren(nid));
            return ids.map(function(cid) { return __cx_wrapElement(cid); });
        },
        get nextSibling() { return null; },
        get className() {
            return __cx_getNodeAttribute(nid, 'class') || '';
        },
        set className(v) {
            __cx_setNodeAttribute(nid, 'class', String(v));
        },
        set id(v) {
            __cx_setNodeAttribute(nid, 'id', String(v));
        },
        get clientWidth() { return parseInt(__cx_getNodeClientSize(nid).split(',')[0]) || 0; },
        get clientHeight() { return parseInt(__cx_getNodeClientSize(nid).split(',')[1]) || 0; },
        get offsetWidth() { return parseInt(__cx_getNodeClientSize(nid).split(',')[0]) || 0; },
        get offsetHeight() { return parseInt(__cx_getNodeClientSize(nid).split(',')[1]) || 0; },
        get offsetLeft() { return parseInt(__cx_getNodeRect(nid).split(',')[0]) || 0; },
        get offsetTop() { return parseInt(__cx_getNodeRect(nid).split(',')[1]) || 0; },
    };
    __cx_elementCache[nid] = el;
    return el;
}

function isDescendant(parentNid, childNid) {
    return true;
}

// ─── Canvas 2D Context Factory ───
function __cx_createContext2D(cid) {
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
            for (var nid in __cx_elementCache) {
                if (__cx_getCanvasId(parseInt(nid)) === cid) return __cx_elementCache[nid];
            }
            return null;
        },
        get fillStyle() { return _fillStyleStr; },
        set fillStyle(v) {
            _fillStyleStr = v;
            if (typeof v === 'string') {
                __cx_c2d_setFillStyle(cid, v);
            } else if (v && v._type === 'gradient') {
                __cx_c2d_setFillGradient(cid, v._id);
            } else if (v && v._type === 'pattern') {
                __cx_c2d_setFillPattern(cid, v._id);
            }
        },
        get strokeStyle() { return _strokeStyleStr; },
        set strokeStyle(v) {
            _strokeStyleStr = v;
            if (typeof v === 'string') {
                __cx_c2d_setStrokeStyle(cid, v);
            } else if (v && v._type === 'gradient') {
                __cx_c2d_setStrokeGradient(cid, v._id);
            }
        },
        get lineWidth() { return _lineWidth; },
        set lineWidth(v) { _lineWidth = v; __cx_c2d(cid, 16, v, 0, 0, 0, 0, 0); },
        get globalAlpha() { return _globalAlpha; },
        set globalAlpha(v) { _globalAlpha = v; __cx_c2d(cid, 17, v, 0, 0, 0, 0, 0); },
        get globalCompositeOperation() { return _globalCompositeOperation; },
        set globalCompositeOperation(v) { _globalCompositeOperation = v; __cx_c2d_setBlendMode(cid, v); },
        get font() { return _font; },
        set font(v) { _font = v; __cx_c2d_setFont(cid, v); },
        get textAlign() { return _textAlign; },
        set textAlign(v) { _textAlign = v; __cx_c2d_setTextAlign(cid, v); },
        get textBaseline() { return _textBaseline; },
        set textBaseline(v) { _textBaseline = v; __cx_c2d_setTextBaseline(cid, v); },
        get lineCap() { return _lineCap; },
        set lineCap(v) { _lineCap = v; },
        get lineJoin() { return _lineJoin; },
        set lineJoin(v) { _lineJoin = v; },
        get shadowBlur() { return _shadowBlur; },
        set shadowBlur(v) { _shadowBlur = Number(v) || 0; },
        get shadowColor() { return _shadowColor; },
        set shadowColor(v) { _shadowColor = String(v); },
        get imageSmoothingEnabled() { return _imageSmoothingEnabled; },
        set imageSmoothingEnabled(v) { _imageSmoothingEnabled = !!v; },
        get miterLimit() { return _miterLimit; },
        set miterLimit(v) { _miterLimit = Number(v) || 10; },
        get lineDashOffset() { return _lineDashOffset; },
        set lineDashOffset(v) { _lineDashOffset = Number(v) || 0; },

        fillRect: function(x, y, w, h) {
            if (_fillStyleStr && typeof _fillStyleStr === 'object') {
                if (_fillStyleStr._type === 'pattern') {
                    __cx_c2d_fillRectPattern(cid, x, y, w, h);
                } else {
                    __cx_c2d_fillRectGrad(cid, x, y, w, h);
                }
            } else {
                __cx_c2d(cid, 1, x, y, w, h, 0, 0);
            }
        },
        strokeRect: function(x, y, w, h) { __cx_c2d(cid, 2, x, y, w, h, 0, 0); },
        clearRect: function(x, y, w, h) { __cx_c2d(cid, 3, x, y, w, h, 0, 0); },
        beginPath: function() { __cx_c2d(cid, 4, 0, 0, 0, 0, 0, 0); },
        closePath: function() { __cx_c2d(cid, 5, 0, 0, 0, 0, 0, 0); },
        moveTo: function(x, y) { __cx_c2d(cid, 6, x, y, 0, 0, 0, 0); },
        lineTo: function(x, y) { __cx_c2d(cid, 7, x, y, 0, 0, 0, 0); },
        arc: function(x, y, r, start, end, ccw) { __cx_c2d(cid, 8, x, y, r, start, end, ccw ? 1 : 0); },
        fill: function() { __cx_c2d(cid, 9, 0, 0, 0, 0, 0, 0); },
        stroke: function() { __cx_c2d(cid, 10, 0, 0, 0, 0, 0, 0); },
        save: function() { __cx_c2d(cid, 11, 0, 0, 0, 0, 0, 0); },
        restore: function() { __cx_c2d(cid, 12, 0, 0, 0, 0, 0, 0); },
        translate: function(x, y) { __cx_c2d(cid, 13, x, y, 0, 0, 0, 0); },
        rotate: function(a) { __cx_c2d(cid, 14, a, 0, 0, 0, 0, 0); },
        scale: function(x, y) { __cx_c2d(cid, 15, x, y, 0, 0, 0, 0); },
        setTransform: function(a, b, c, d, e, f) { __cx_c2d(cid, 20, a, b, c, d, e, f); },
        resetTransform: function() { __cx_c2d(cid, 21, 0, 0, 0, 0, 0, 0); },
        bezierCurveTo: function(cp1x, cp1y, cp2x, cp2y, x, y) { __cx_c2d(cid, 18, cp1x, cp1y, cp2x, cp2y, x, y); },
        quadraticCurveTo: function(cpx, cpy, x, y) { __cx_c2d(cid, 19, cpx, cpy, x, y, 0, 0); },

        drawImage: function(source) {
            if (!source || !source._nid) return;
            var srcCid = __cx_getCanvasId(source._nid);
            if (srcCid < 0) return;
            if (arguments.length >= 5) {
                __cx_c2d_drawImage(cid, srcCid, arguments[1], arguments[2], arguments[3], arguments[4]);
            } else if (arguments.length >= 3) {
                var size = __cx_canvasGetSize(srcCid).split(',');
                __cx_c2d_drawImage(cid, srcCid, arguments[1], arguments[2], parseInt(size[0]), parseInt(size[1]));
            }
        },

        createRadialGradient: function(x0, y0, r0, x1, y1, r1) {
            var gid = __cx_c2d_createRadialGradient(x0, y0, r0, x1, y1, r1);
            return {
                _type: 'gradient',
                _id: gid,
                addColorStop: function(offset, color) {
                    __cx_c2d_gradientAddStop(gid, offset, color);
                }
            };
        },
        createLinearGradient: function(x0, y0, x1, y1) {
            var gid = __cx_c2d_createLinearGradient(x0, y0, x1, y1);
            return {
                _type: 'gradient',
                _id: gid,
                addColorStop: function(offset, color) {
                    __cx_c2d_gradientAddStop(gid, offset, color);
                }
            };
        },
        createPattern: function(source, repeat) {
            if (!source || !source._nid) return null;
            var srcCid = __cx_getCanvasId(source._nid);
            if (srcCid < 0) return null;
            return {
                _type: 'pattern',
                _id: srcCid,
            };
        },
        fillText: function(text, x, y) { __cx_c2d_fillText(cid, text, x, y); },
        strokeText: function() {},
        measureText: function(text) { return { width: text.length * 7 }; },
        getImageData: function(x, y, w, h) {
            var raw = __cx_c2d_getImageData(cid, x|0, y|0, w|0, h|0);
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
            __cx_c2d_putImageData(
                cid,
                x|0,
                y|0,
                (imageData.width || 0)|0,
                (imageData.height || 0)|0,
                JSON.stringify(Array.prototype.slice.call(imageData.data))
            );
        },
        clip: function() { __cx_c2d_clipPath(cid); },
        setLineDash: function(segments) {},
        getLineDash: function() { return []; },
    };
    return ctx;
}

// ─── document object ───
var document = {
    getElementById: function(id) {
        var nid = __cx_getElementById(id);
        return __cx_wrapElement(nid);
    },
    querySelector: function(sel) {
        var ids = JSON.parse(__cx_querySelectorAll(sel));
        if (ids.length > 0) return __cx_wrapElement(ids[0]);
        return null;
    },
    querySelectorAll: function(sel) {
        var ids = JSON.parse(__cx_querySelectorAll(sel));
        return ids.map(function(id) { return __cx_wrapElement(id); });
    },
    createElement: function(tag) {
        var nid = __cx_createElement(tag);
        return __cx_wrapElement(nid);
    },
    get documentElement() {
        return __cx_wrapElement(0);
    },
    get body() {
        return __cx_wrapElement(0);
    },
};

// ─── window object ───
var window = (typeof globalThis !== 'undefined') ? globalThis : {};
window.document = document;
var __cx_viewport = __cx_getViewportSize().split(',');
window.innerWidth = parseInt(__cx_viewport[0]) || 1920;
window.innerHeight = parseInt(__cx_viewport[1]) || 1080;
window.top = window;
window.self = window;
window.parent = window;

var __cx_globalListeners = {};
function addEventListener(type, fn) {
    if (!__cx_globalListeners[type]) __cx_globalListeners[type] = [];
    __cx_globalListeners[type].push(fn);
}
function removeEventListener(type, fn) {
    if (!__cx_globalListeners[type]) return;
    var idx = __cx_globalListeners[type].indexOf(fn);
    if (idx >= 0) __cx_globalListeners[type].splice(idx, 1);
}
window.addEventListener = addEventListener;
window.removeEventListener = removeEventListener;

// ─── console ───
var console = {
    log: function() { __cx_log(1, Array.prototype.slice.call(arguments).join(' ')); },
    warn: function() { __cx_log(2, Array.prototype.slice.call(arguments).join(' ')); },
    error: function() { __cx_log(3, Array.prototype.slice.call(arguments).join(' ')); },
    info: function() { __cx_log(1, Array.prototype.slice.call(arguments).join(' ')); },
    debug: function() { __cx_log(0, Array.prototype.slice.call(arguments).join(' ')); },
    dumpDoc: function() { __cx_dumpDoc(); },
};

// ─── performance ───
var performance = {
    now: function() { return __cx_performance_now() - __cx_startTime; },
};

// ─── requestAnimationFrame ───
function requestAnimationFrame(cb) {
    var id = ++__cx_rafId;
    __cx_rafCallbacks.push({ id: id, callback: cb });
    return id;
}
function cancelAnimationFrame(id) {
    __cx_rafCallbacks = __cx_rafCallbacks.filter(function(c) { return c.id !== id; });
}

// ─── setTimeout / setInterval ───
function setTimeout(fn, delay) {
    var id = __cx_nextTimerId++;
    var triggerAt = __cx_performance_now() + (delay || 0);
    __cx_timeouts.push({ id: id, callback: fn, triggerAt: triggerAt });
    return id;
}
function clearTimeout(id) {
    __cx_timeouts = __cx_timeouts.filter(function(t) { return t.id !== id; });
}
function setInterval(fn, delay) {
    var id = __cx_nextTimerId++;
    var interval = delay || 16;
    var triggerAt = __cx_performance_now() + interval;
    __cx_intervals.push({ id: id, callback: fn, interval: interval, triggerAt: triggerAt });
    return id;
}
function clearInterval(id) {
    __cx_intervals = __cx_intervals.filter(function(t) { return t.id !== id; });
}

// ─── getComputedStyle ───
function getComputedStyle(el) {
    return {
        getPropertyValue: function(name) {
            if (name.startsWith('--')) return __cx_getComputedStyleVar(name);
            if (el && el._nid !== undefined) return __cx_getStyle(el._nid, name);
            return '';
        }
    };
}

// ─── rAF tick function (called by Rust each frame) ───
function __cx_raf_tick(timestamp) {
    var now = __cx_performance_now();
    var pendingTimeouts = [];
    var remaining = [];
    for (var i = 0; i < __cx_timeouts.length; i++) {
        if (now >= __cx_timeouts[i].triggerAt) {
            pendingTimeouts.push(__cx_timeouts[i]);
        } else {
            remaining.push(__cx_timeouts[i]);
        }
    }
    __cx_timeouts = remaining;
    for (var j = 0; j < pendingTimeouts.length; j++) {
        try { pendingTimeouts[j].callback(); } catch(e) { console.error('Timeout error:', e); }
    }

    for (var k = 0; k < __cx_intervals.length; k++) {
        if (now >= __cx_intervals[k].triggerAt) {
            try { __cx_intervals[k].callback(); } catch(e) { console.error('Interval error:', e); }
            __cx_intervals[k].triggerAt = now + __cx_intervals[k].interval;
        }
    }

    var callbacks = __cx_rafCallbacks.slice();
    __cx_rafCallbacks = [];
    for (var m = 0; m < callbacks.length; m++) {
        var entry = callbacks[m];
        if (typeof entry.callback !== 'function') {
            console.error('[CX] rAF entry not callable: type=' + typeof entry.callback + ' id=' + entry.id);
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
    var __vp = __cx_getViewportSize().split(',');
    window.innerWidth = parseInt(__vp[0]) || 1920;
    window.innerHeight = parseInt(__vp[1]) || 1080;
} catch(e) {}
"#;
