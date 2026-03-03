// canvasx-runtime/src/scripting/runtime.rs
//
// JavaScript runtime powered by boa_engine.
// Manages JS context, DOM bindings, Canvas 2D, requestAnimationFrame, and IPC bridge.
//
// The runtime injects a JS shim that wraps native Rust functions into standard
// web APIs (document, Canvas2D, performance, console, etc.).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;

use boa_engine::{Context, JsArgs, JsResult, JsValue, NativeFunction, Source};
use boa_engine::js_string;

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::{NodeKind, NodeId};
use crate::compiler::css::{CssRule, apply_property};
use crate::ipc::client::send_ipc_request_to;
use crate::ipc::protocol::IpcRequest;
use crate::scripting::canvas2d::{CanvasManager, CanvasId, GradientDef, parse_css_color};

thread_local! {
    static RUNTIME_STATE: RefCell<Option<StateRef>> = RefCell::new(None);
}

fn with_state<F, R>(f: F) -> JsResult<R>
where
    F: FnOnce(&mut SharedState) -> JsResult<R>,
{
    RUNTIME_STATE.with(|cell| {
        let opt = cell.borrow();
        let state_ref = opt.as_ref().expect("RUNTIME_STATE not set");
        let mut state = state_ref.borrow_mut();
        f(&mut state)
    })
}

/// Shared mutable state accessible from both Rust and JS native functions.
pub struct SharedState {
    pub document: CxrdDocument,
    pub css_rules: Vec<CssRule>,
    pub css_variables: HashMap<String, String>,
    pub canvas_manager: CanvasManager,
    /// Map from CxrdNode ID → CanvasId (for <canvas> elements in the DOM).
    pub node_canvas_map: HashMap<NodeId, CanvasId>,
    /// Pending layout invalidation flag.
    pub layout_dirty: bool,
    /// Accumulated requestAnimationFrame callback IDs.
    pub raf_pending: bool,
    /// Data values from IPC (populated by sentinel.js).
    pub data_values: HashMap<String, String>,
}

pub type StateRef = Rc<RefCell<SharedState>>;

/// The JavaScript runtime.
pub struct JsRuntime {
    context: Context,
    pub state: StateRef,
    initialized: bool,
}

impl JsRuntime {
    /// Create a new JS runtime and register all native bindings.
    pub fn new(
        document: CxrdDocument,
        css_rules: Vec<CssRule>,
        css_variables: HashMap<String, String>,
    ) -> Self {
        let state = Rc::new(RefCell::new(SharedState {
            document,
            css_rules,
            css_variables,
            canvas_manager: CanvasManager::new(),
            node_canvas_map: HashMap::new(),
            layout_dirty: false,
            raf_pending: false,
            data_values: HashMap::new(),
        }));

        RUNTIME_STATE.with(|cell| {
            *cell.borrow_mut() = Some(state.clone());
        });

        let mut context = Context::default();

        // Register all native functions
        register_console(&mut context);
        register_performance(&mut context);
        register_dom(&mut context);
        register_canvas2d(&mut context);
        register_ipc(&mut context);
        register_timers(&mut context);
        register_misc(&mut context);

        // Inject the JavaScript DOM/Canvas shim
        log::warn!("[CX][JS] Injecting JS shim ({} bytes)", JS_SHIM.len());
        let shim_result = context.eval(Source::from_bytes(JS_SHIM.as_bytes()));
        match &shim_result {
            Ok(_) => log::warn!("[CX][JS] JS shim injected OK"),
            Err(e) => log::error!("[CX][JS] JS shim injection FAILED: {}", e),
        }

        Self {
            context,
            state,
            initialized: false,
        }
    }

    /// Initialize canvas elements — create CanvasBuffer for each <canvas> node.
    pub fn init_canvases(&mut self, viewport_width: u32, viewport_height: u32) {
        let mut state = self.state.borrow_mut();
        let count = state.document.nodes.len();
        for i in 0..count {
            if state.document.nodes[i].tag.as_deref() == Some("canvas") {
                // Get canvas dimensions from layout or default to viewport size
                let w = if state.document.nodes[i].layout.rect.width > 0.0 {
                    state.document.nodes[i].layout.rect.width as u32
                } else {
                    viewport_width
                };
                let h = if state.document.nodes[i].layout.rect.height > 0.0 {
                    state.document.nodes[i].layout.rect.height as u32
                } else {
                    viewport_height
                };
                let node_id = state.document.nodes[i].id;
                let canvas_id = state.canvas_manager.create_canvas(w, h);
                state.node_canvas_map.insert(node_id, canvas_id);
                log::info!("Created canvas buffer {} for node {} ({}×{})", canvas_id, node_id, w, h);
            }
        }
    }

    /// Execute script source code.
    pub fn execute(&mut self, source: &str, name: &str) {
        log::warn!("[CX][JS] Executing script '{}' ({} bytes)", name, source.len());
        let result = self.context.eval(Source::from_bytes(source.as_bytes()));
        match result {
            Ok(_) => {
                log::warn!("[CX][JS] Script '{}' completed OK", name);
                // Dump document state after script execution
                let state = self.state.borrow();
                let node_count = state.document.nodes.len();
                let canvas_count = state.node_canvas_map.len();
                let has_ids: Vec<String> = state.document.nodes.iter()
                    .filter_map(|n| n.html_id.as_ref().map(|id| format!("{}=node{}", id, n.id)))
                    .collect();
                log::warn!("[CX][JS] Doc state: {} nodes, {} canvases, ids=[{}], layout_dirty={}",
                    node_count, canvas_count, has_ids.join(", "), state.layout_dirty);
            }
            Err(e) => log::error!("[CX][JS] Script '{}' THREW: {}", name, e),
        }
    }

    /// Execute a script file from disk.
    pub fn execute_file(&mut self, path: &Path) {
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

    /// Run one frame tick: execute all pending requestAnimationFrame callbacks.
    /// Returns true if any canvas was modified (needs GPU texture re-upload).
    pub fn tick(&mut self, dt: f32) -> bool {
        // Set the current timestamp for performance.now()
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as f64;

        // Call __cx_raf_tick(timestamp) which fires pending rAF callbacks
        let tick_code = format!("if(typeof __cx_raf_tick==='function')__cx_raf_tick({});", now_ms);
        match self.context.eval(Source::from_bytes(tick_code.as_bytes())) {
            Ok(_) => {},
            Err(e) => log::error!("[CX][JS] tick error: {}", e),
        }

        // Check if any canvas was dirtied
        let state = self.state.borrow();
        state.canvas_manager.buffers.values().any(|b| b.dirty)
    }

    /// Mark all canvas buffers as clean (after GPU upload).
    pub fn clear_dirty_flags(&mut self) {
        let mut state = self.state.borrow_mut();
        for canvas in state.canvas_manager.buffers.values_mut() {
            canvas.dirty = false;
        }
    }

    /// Get all dirty canvas buffers for GPU texture upload.
    /// Returns Vec<(CanvasId, NodeId, width, height, pixel_data)>.
    pub fn dirty_canvases(&self) -> Vec<(CanvasId, Option<NodeId>, u32, u32, Vec<u8>)> {
        let state = self.state.borrow();
        let mut result = Vec::new();
        for (&cid, canvas) in &state.canvas_manager.buffers {
            if canvas.dirty {
                // Find the node ID that owns this canvas
                let node_id = state.node_canvas_map.iter()
                    .find(|(_, &v)| v == cid)
                    .map(|(&k, _)| k);
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

    /// Re-apply CSS rules to the entire document tree using compile-time
    /// pipeline (tag defaults → parent inherit → compound selector matching
    /// → inline styles).  Call after JS has finished mutating the DOM.
    pub fn restyle(&self) {
        let mut state = self.state.borrow_mut();
        let rules = state.css_rules.clone();
        let vars = state.css_variables.clone();
        crate::compiler::html::restyle_document(&mut state.document, &rules, &vars);
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// Native function registration
// ═══════════════════════════════════════════════════════════════════════════

/// Register console.log / console.warn / console.error.
fn register_console(ctx: &mut Context) {
    ctx.register_global_builtin_callable(
        js_string!("__cx_log"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let level = args.get_or_undefined(0).to_i32(context).unwrap_or(0);
            let msg = args.get_or_undefined(1).to_string(context)
                .map(|s: boa_engine::JsString| s.to_std_string_escaped())
                .unwrap_or_default();
            match level {
                0 => log::debug!("[JS] {}", msg),
                1 => log::info!("[JS] {}", msg),
                2 => log::warn!("[JS] {}", msg),
                3 => log::error!("[JS] {}", msg),
                _ => log::info!("[JS] {}", msg),
            }
            Ok(JsValue::undefined())
        }),
    ).unwrap();
}

/// Register performance.now().
fn register_performance(ctx: &mut Context) {
    ctx.register_global_builtin_callable(
        js_string!("__cx_performance_now"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _ctx| {
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs_f64() * 1000.0;
            Ok(JsValue::from(now))
        }),
    ).unwrap();
}

/// Register setTimeout / setInterval stubs and requestAnimationFrame.
fn register_timers(ctx: &mut Context) {
    // requestAnimationFrame is handled in the JS shim — __cx_raf_tick dispatches callbacks
    // setTimeout/setInterval are also handled in JS shim with simplified behavior
}

/// Register DOM native functions.
fn register_dom(ctx: &mut Context) {
    // __cx_getElementById(id) → node_id or -1
    ctx.register_global_builtin_callable(
        js_string!("__cx_getElementById"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let id = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                for node in &st.document.nodes {
                    if node.html_id.as_deref() == Some(id.as_str()) {
                        log::info!("[CX][DOM] getElementById('{}') → node {}", id, node.id);
                        return Ok(JsValue::from(node.id as i32));
                    }
                }
                log::warn!("[CX][DOM] getElementById('{}') → NOT FOUND (doc has {} nodes)", id, st.document.nodes.len());
                Ok(JsValue::from(-1_i32))
            })
        }),
    ).unwrap();

    // __cx_querySelector(selector) → node_id or -1
    ctx.register_global_builtin_callable(
        js_string!("__cx_querySelector"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let selector = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                // Simple selector matching: .class, #id, tag
                for node in &st.document.nodes {
                    if selector_matches_node(&selector, node, &st.document) {
                        return Ok(JsValue::from(node.id as i32));
                    }
                }
                Ok(JsValue::from(-1_i32))
            })
        }),
    ).unwrap();

    // __cx_querySelectorAll(selector) → JSON array of node IDs
    ctx.register_global_builtin_callable(
        js_string!("__cx_querySelectorAll"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let selector = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                let mut ids = Vec::new();
                for node in &st.document.nodes {
                    if selector_matches_node(&selector, node, &st.document) {
                        ids.push(node.id.to_string());
                    }
                }
                let json = format!("[{}]", ids.join(","));
                Ok(JsValue::from(js_string!(json)))
            })
        }),
    ).unwrap();

    // __cx_createElement(tag) → new node_id
    ctx.register_global_builtin_callable(
        js_string!("__cx_createElement"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let tag = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                let mut node = crate::cxrd::node::CxrdNode::container(0);
                node.tag = Some(tag.clone());
                // For canvas elements, create a canvas buffer
                let node_id = st.document.add_node(node);
                if tag == "canvas" {
                    let cid = st.canvas_manager.create_canvas(300, 150); // default HTML canvas size
                    st.node_canvas_map.insert(node_id, cid);
                    log::info!("[CX][DOM] createElement('canvas') → node={} canvas={}", node_id, cid);
                } else {
                    log::info!("[CX][DOM] createElement('{}') → node={}", tag, node_id);
                }
                Ok(JsValue::from(node_id as i32))
            })
        }),
    ).unwrap();

    // __cx_getTextContent(nid) → string
    ctx.register_global_builtin_callable(
        js_string!("__cx_getTextContent"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    match &node.kind {
                        NodeKind::Text { content } => return Ok(JsValue::from(js_string!(content.clone()))),
                        _ => {
                            // Collect text from children
                            let text = collect_text_content(&st.document, nid);
                            return Ok(JsValue::from(js_string!(text)));
                        }
                    }
                }
                Ok(JsValue::from(js_string!("")))
            })
        }),
    ).unwrap();

    // __cx_setTextContent(nid, text)
    ctx.register_global_builtin_callable(
        js_string!("__cx_setTextContent"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let text = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                // Remove existing children and set node content to text
                if let Some(node) = st.document.get_node_mut(nid) {
                    node.children.clear();
                    node.kind = NodeKind::Text { content: text };
                    st.layout_dirty = true;
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_setInnerHTML(nid, html)
    ctx.register_global_builtin_callable(
        js_string!("__cx_setInnerHTML"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let html = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                log::info!("[CX][DOM] setInnerHTML: node={} html_len={} html_preview='{}'", nid, html.len(), &html[..html.len().min(120)]);
                set_inner_html(st, nid, &html);
                log::info!("[CX][DOM] setInnerHTML done: total nodes={}", st.document.nodes.len());
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getNodeAttribute(nid, name) → string
    ctx.register_global_builtin_callable(
        js_string!("__cx_getNodeAttribute"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let name = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    if let Some(val) = node.attributes.get(&name) {
                        return Ok(JsValue::from(js_string!(val.clone())));
                    }
                }
                Ok(JsValue::null())
            })
        }),
    ).unwrap();

    // __cx_setNodeAttribute(nid, name, value)
    ctx.register_global_builtin_callable(
        js_string!("__cx_setNodeAttribute"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let name = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
            let value = args.get_or_undefined(2).to_string(context)?.to_std_string_escaped();
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
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_classListOp(nid, op, className) → bool
    // op: 0=add, 1=remove, 2=toggle, 3=contains
    ctx.register_global_builtin_callable(
        js_string!("__cx_classListOp"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let op = args.get_or_undefined(1).to_i32(context)?;
            let class_name = args.get_or_undefined(2).to_string(context)?
                .to_std_string_escaped();
            let force = if args.len() > 3 {
                let v = args.get_or_undefined(3);
                if v.is_undefined() { None } else { Some(v.to_boolean()) }
            } else {
                None
            };

            with_state(|st| {
                let result = if let Some(node) = st.document.get_node_mut(nid) {
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
                        3 => { // contains
                            node.classes.contains(&class_name)
                        }
                        _ => false,
                    }
                } else {
                    false
                };
                Ok(JsValue::from(result))
            })
        }),
    ).unwrap();

    // __cx_getStyle(nid, prop) → string
    ctx.register_global_builtin_callable(
        js_string!("__cx_getStyle"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let prop = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    let val = get_computed_style_value(&node.style, &prop);
                    return Ok(JsValue::from(js_string!(val)));
                }
                Ok(JsValue::from(js_string!("")))
            })
        }),
    ).unwrap();

    // __cx_setStyle(nid, prop, value)
    ctx.register_global_builtin_callable(
        js_string!("__cx_setStyle"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let prop = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
            let value = args.get_or_undefined(2).to_string(context)?.to_std_string_escaped();
            with_state(|st| {
                let variables = st.css_variables.clone();
                if let Some(node) = st.document.get_node_mut(nid) {
                    apply_property(&mut node.style, &prop, &value, &variables);
                    st.layout_dirty = true;
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getComputedStyleVar(prop) → string (reads CSS variable from :root)
    ctx.register_global_builtin_callable(
        js_string!("__cx_getComputedStyleVar"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let prop = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(val) = st.css_variables.get(&prop) {
                    return Ok(JsValue::from(js_string!(val.clone())));
                }
                Ok(JsValue::from(js_string!("")))
            })
        }),
    ).unwrap();

    // __cx_setRootStyleProperty(prop, value) — set a CSS variable on :root
    ctx.register_global_builtin_callable(
        js_string!("__cx_setRootStyleProperty"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let prop = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
            let value = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
            with_state(|st| {
                st.css_variables.insert(prop, value);
                st.layout_dirty = true;
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getNodeTag(nid) → string
    ctx.register_global_builtin_callable(
        js_string!("__cx_getNodeTag"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    if let Some(tag) = &node.tag {
                        return Ok(JsValue::from(js_string!(tag.to_uppercase())));
                    }
                }
                Ok(JsValue::from(js_string!("DIV")))
            })
        }),
    ).unwrap();

    // __cx_getNodeChildren(nid) → JSON array of child node IDs
    ctx.register_global_builtin_callable(
        js_string!("__cx_getNodeChildren"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    let ids: Vec<String> = node.children.iter().map(|c| c.to_string()).collect();
                    let json = format!("[{}]", ids.join(","));
                    return Ok(JsValue::from(js_string!(json)));
                }
                Ok(JsValue::from(js_string!("[]")))
            })
        }),
    ).unwrap();

    // __cx_getNodeId(nid) → html id string or ""
    ctx.register_global_builtin_callable(
        js_string!("__cx_getNodeId"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    if let Some(html_id) = &node.html_id {
                        return Ok(JsValue::from(js_string!(html_id.clone())));
                    }
                }
                Ok(JsValue::from(js_string!("")))
            })
        }),
    ).unwrap();

    // __cx_appendChild(parent_nid, child_nid) — move child under parent
    ctx.register_global_builtin_callable(
        js_string!("__cx_appendChild"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let parent_nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let child_nid = args.get_or_undefined(1).to_i32(context)? as u32;
            with_state(|st| {
                // Remove from current parent if any
                if let Some(old_parent) = st.document.find_parent(child_nid) {
                    st.document.remove_child(old_parent, child_nid);
                }
                st.document.add_child(parent_nid, child_nid);
                log::info!("[CX][DOM] appendChild: parent={} child={} (total nodes={})", parent_nid, child_nid, st.document.nodes.len());
                // Re-apply CSS rules on the moved node
                let css_rules = st.css_rules.clone();
                let css_variables = st.css_variables.clone();
                if let Some(node) = st.document.get_node_mut(child_nid) {
                    for rule in &css_rules {
                        if simple_rule_matches(&rule.selector, node) {
                            for (prop, val) in &rule.declarations {
                                apply_property(&mut node.style, prop, val, &css_variables);
                            }
                        }
                    }
                }
                st.layout_dirty = true;
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_removeChild(parent_nid, child_nid)
    ctx.register_global_builtin_callable(
        js_string!("__cx_removeChild"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let parent_nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let child_nid = args.get_or_undefined(1).to_i32(context)? as u32;
            with_state(|st| {
                st.document.remove_child(parent_nid, child_nid);
                st.layout_dirty = true;
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getParentNode(nid) → parent_nid or -1
    ctx.register_global_builtin_callable(
        js_string!("__cx_getParentNode"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(parent_id) = st.document.find_parent(nid) {
                    return Ok(JsValue::from(parent_id as i32));
                }
                Ok(JsValue::from(-1_i32))
            })
        }),
    ).unwrap();

    // __cx_insertBefore(parent_nid, new_nid, ref_nid)
    // If ref_nid < 0, prepend (insert at position 0)
    ctx.register_global_builtin_callable(
        js_string!("__cx_insertBefore"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let parent_nid = args.get_or_undefined(0).to_i32(context)? as u32;
            let new_nid = args.get_or_undefined(1).to_i32(context)? as u32;
            let ref_nid = args.get_or_undefined(2).to_i32(context)?;
            with_state(|st| {
                // Remove from current parent if any
                if let Some(old_parent) = st.document.find_parent(new_nid) {
                    st.document.remove_child(old_parent, new_nid);
                }
                if let Some(parent) = st.document.get_node_mut(parent_nid) {
                    if ref_nid < 0 {
                        // Prepend
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
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getNodeClientSize(nid) → "width,height"
    ctx.register_global_builtin_callable(
        js_string!("__cx_getNodeClientSize"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(node) = st.document.get_node(nid) {
                    let w = node.layout.rect.width;
                    let h = node.layout.rect.height;
                    let size = format!("{},{}", w as i32, h as i32);
                    return Ok(JsValue::from(js_string!(size)));
                }
                Ok(JsValue::from(js_string!("0,0")))
            })
        }),
    ).unwrap();
}

/// Register Canvas 2D native functions.
fn register_canvas2d(ctx: &mut Context) {
    // __cx_getCanvasId(nid) → canvas_id or -1
    ctx.register_global_builtin_callable(
        js_string!("__cx_getCanvasId"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let nid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(&cid) = st.node_canvas_map.get(&nid) {
                    log::info!("[CX][Canvas] getCanvasId(node={}) → canvas {}", nid, cid);
                    return Ok(JsValue::from(cid as i32));
                }
                log::warn!("[CX][Canvas] getCanvasId(node={}) → NOT FOUND (map has {} entries: {:?})", nid, st.node_canvas_map.len(), st.node_canvas_map);
                Ok(JsValue::from(-1_i32))
            })
        }),
    ).unwrap();

    // __cx_canvasSetSize(cid, w, h)
    ctx.register_global_builtin_callable(
        js_string!("__cx_canvasSetSize"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let w = args.get_or_undefined(1).to_i32(context)? as u32;
            let h = args.get_or_undefined(2).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.resize(w, h);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_canvasGetSize(cid) → "w,h"
    ctx.register_global_builtin_callable(
        js_string!("__cx_canvasGetSize"),
        1,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get(&cid) {
                    let size = format!("{},{}", canvas.width, canvas.height);
                    return Ok(JsValue::from(js_string!(size)));
                }
                Ok(JsValue::from(js_string!("0,0")))
            })
        }),
    ).unwrap();

    // Canvas 2D drawing commands — use a single dispatcher to reduce registration count
    // __cx_c2d(cid, cmd, a1, a2, a3, a4, a5, a6)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d"),
        8,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let cmd = args.get_or_undefined(1).to_i32(context)?;
            let a1 = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let a2 = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            let a3 = args.get_or_undefined(4).to_number(context).unwrap_or(0.0) as f32;
            let a4 = args.get_or_undefined(5).to_number(context).unwrap_or(0.0) as f32;
            let a5 = args.get_or_undefined(6).to_number(context).unwrap_or(0.0) as f32;
            let a6 = args.get_or_undefined(7).to_number(context).unwrap_or(0.0) as f32;

            with_state(|st| {
                // Handle fill/stroke commands that need gradient resolution first
                if cmd == 9 {
                    let uses_gradient = st.canvas_manager.buffers.get(&cid)
                        .map(|c| c.uses_gradient_fill())
                        .unwrap_or(false);
                    if uses_gradient {
                        st.canvas_manager.fill_path_with_gradient(cid);
                    } else if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                        canvas.fill();
                    }
                    return Ok(JsValue::undefined());
                }
                if cmd == 10 {
                    let uses_gradient = st.canvas_manager.buffers.get(&cid)
                        .map(|c| c.uses_gradient_stroke())
                        .unwrap_or(false);
                    if uses_gradient {
                        st.canvas_manager.stroke_path_with_gradient(cid);
                    } else if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                        canvas.stroke();
                    }
                    return Ok(JsValue::undefined());
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
                        _ => {}
                    }
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setFillStyle(cid, colorStr)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setFillStyle"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let color_str = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(color) = parse_css_color(&color_str) {
                    if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                        canvas.set_fill_style_color(color);
                    }
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setStrokeStyle(cid, colorStr)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setStrokeStyle"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let color_str = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(color) = parse_css_color(&color_str) {
                    if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                        canvas.set_stroke_style_color(color);
                    }
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setFillGradient(cid, gradientId)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setFillGradient"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let gid = args.get_or_undefined(1).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.set_fill_style_gradient(gid);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setStrokeGradient(cid, gradientId)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setStrokeGradient"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let gid = args.get_or_undefined(1).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.set_stroke_style_gradient(gid);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setFillPattern(cid, patternCanvasId)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setFillPattern"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let pid = args.get_or_undefined(1).to_i32(context)? as u32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.set_fill_style_pattern(pid);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setBlendMode(cid, modeStr)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setBlendMode"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let mode = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.set_blend_mode(&mode);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_setFont(cid, fontStr)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_setFont"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let font = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.set_font(&font);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_fillText(cid, text, x, y)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_fillText"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let text = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
            let x = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let y = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                if let Some(canvas) = st.canvas_manager.buffers.get_mut(&cid) {
                    canvas.fill_text(&text, x, y);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_drawImage(targetCid, sourceCid, dx, dy, dw, dh)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_drawImage"),
        6,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let target_cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let source_cid = args.get_or_undefined(1).to_i32(context)? as u32;
            let dx = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let dy = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            let dw = args.get_or_undefined(4).to_number(context).unwrap_or(0.0) as f32;
            let dh = args.get_or_undefined(5).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                st.canvas_manager.draw_canvas_to_canvas(target_cid, source_cid, dx, dy, dw, dh);
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_createRadialGradient(x0, y0, r0, x1, y1, r1) → gradientId
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_createRadialGradient"),
        6,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let x0 = args.get_or_undefined(0).to_number(context).unwrap_or(0.0) as f32;
            let y0 = args.get_or_undefined(1).to_number(context).unwrap_or(0.0) as f32;
            let r0 = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let x1 = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            let y1 = args.get_or_undefined(4).to_number(context).unwrap_or(0.0) as f32;
            let r1 = args.get_or_undefined(5).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                let gid = st.canvas_manager.create_gradient(GradientDef::Radial {
                    x0, y0, r0, x1, y1, r1, stops: Vec::new(),
                });
                Ok(JsValue::from(gid as i32))
            })
        }),
    ).unwrap();

    // __cx_c2d_createLinearGradient(x0, y0, x1, y1) → gradientId
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_createLinearGradient"),
        4,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let x0 = args.get_or_undefined(0).to_number(context).unwrap_or(0.0) as f32;
            let y0 = args.get_or_undefined(1).to_number(context).unwrap_or(0.0) as f32;
            let x1 = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let y1 = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                let gid = st.canvas_manager.create_gradient(GradientDef::Linear {
                    x0, y0, x1, y1, stops: Vec::new(),
                });
                Ok(JsValue::from(gid as i32))
            })
        }),
    ).unwrap();

    // __cx_c2d_gradientAddStop(gradientId, offset, colorStr)
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_gradientAddStop"),
        3,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let gid = args.get_or_undefined(0).to_i32(context)? as u32;
            let offset = args.get_or_undefined(1).to_number(context).unwrap_or(0.0) as f32;
            let color_str = args.get_or_undefined(2).to_string(context)?
                .to_std_string_escaped();
            with_state(|st| {
                if let Some(color) = parse_css_color(&color_str) {
                    st.canvas_manager.add_gradient_stop(gid, offset, color);
                }
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_createPattern(sourceCid, repeat) → patternCanvasId
    // For now, patterns use the source canvas ID directly
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_createPattern"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let source_cid = args.get_or_undefined(0).to_i32(context)?;
            // Pattern is identified by the source canvas ID
            Ok(JsValue::from(source_cid))
        }),
    ).unwrap();

    // __cx_c2d_fillRectGrad(cid, x, y, w, h) — fill rect with current gradient fill
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_fillRectGrad"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let x = args.get_or_undefined(1).to_number(context).unwrap_or(0.0) as f32;
            let y = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let w = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            let h = args.get_or_undefined(4).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                st.canvas_manager.fill_rect_with_gradient(cid, x, y, w, h);
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_c2d_fillRectPattern(cid, x, y, w, h) — fill rect with current pattern fill
    ctx.register_global_builtin_callable(
        js_string!("__cx_c2d_fillRectPattern"),
        5,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let cid = args.get_or_undefined(0).to_i32(context)? as u32;
            let x = args.get_or_undefined(1).to_number(context).unwrap_or(0.0) as f32;
            let y = args.get_or_undefined(2).to_number(context).unwrap_or(0.0) as f32;
            let w = args.get_or_undefined(3).to_number(context).unwrap_or(0.0) as f32;
            let h = args.get_or_undefined(4).to_number(context).unwrap_or(0.0) as f32;
            with_state(|st| {
                st.canvas_manager.fill_rect_with_pattern(cid, x, y, w, h);
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();
}

/// Register the IPC bridge.
fn register_ipc(ctx: &mut Context) {
    // __cx_ipc_send(pipeName, requestJson) → responseJson
    ctx.register_global_builtin_callable(
        js_string!("__cx_ipc_send"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let pipe_name = args.get_or_undefined(0).to_string(context)?
                .to_std_string_escaped();
            let request_json = args.get_or_undefined(1).to_string(context)?
                .to_std_string_escaped();

            // Parse the request JSON
            let request: IpcRequest = match serde_json::from_str(&request_json) {
                Ok(r) => r,
                Err(e) => {
                    let err_json = format!(r#"{{"ok":false,"error":"Parse error: {}"}}"#, e);
                    return Ok(JsValue::from(js_string!(err_json)));
                }
            };

            // Send the request
            match send_ipc_request_to(&pipe_name, request) {
                Ok(resp) => {
                    let json = serde_json::to_string(&resp).unwrap_or_else(|_| {
                        r#"{"ok":false,"error":"Serialize error"}"#.to_string()
                    });
                    Ok(JsValue::from(js_string!(json)))
                }
                Err(e) => {
                    let err_json = format!(r#"{{"ok":false,"error":"{}"}}"#, e.replace('"', "'"));
                    Ok(JsValue::from(js_string!(err_json)))
                }
            }
        }),
    ).unwrap();
}

/// Register miscellaneous globals.
fn register_misc(ctx: &mut Context) {
    // __cx_setDataValue(key, value) — set a data value for data-bind elements
    ctx.register_global_builtin_callable(
        js_string!("__cx_setDataValue"),
        2,
        NativeFunction::from_fn_ptr(|_this, args, context| {
            let key = args.get_or_undefined(0).to_string(context)?.to_std_string_escaped();
            let value = args.get_or_undefined(1).to_string(context)?.to_std_string_escaped();
            with_state(|st| {
                st.data_values.insert(key, value);
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();

    // __cx_getViewportSize() → "w,h"
    ctx.register_global_builtin_callable(
        js_string!("__cx_getViewportSize"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
            with_state(|st| {
                let root = &st.document.nodes[st.document.root as usize];
                let w = root.layout.rect.width;
                let h = root.layout.rect.height;
                let size = format!("{},{}", w as i32, h as i32);
                Ok(JsValue::from(js_string!(size)))
            })
        }),
    ).unwrap();

    // __cx_dumpDoc() — dump the full document node tree to the log for debugging
    ctx.register_global_builtin_callable(
        js_string!("__cx_dumpDoc"),
        0,
        NativeFunction::from_fn_ptr(|_this, _args, _context| {
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
                Ok(JsValue::undefined())
            })
        }),
    ).unwrap();
}

// ═══════════════════════════════════════════════════════════════════════════
// Helper functions
// ═══════════════════════════════════════════════════════════════════════════

/// Simple selector matching for querySelector.
fn selector_matches_node(selector: &str, node: &crate::cxrd::node::CxrdNode, _doc: &CxrdDocument) -> bool {
    let sel = selector.trim();
    if sel.starts_with('#') {
        let id = &sel[1..];
        return node.html_id.as_deref() == Some(id);
    }
    if sel.starts_with('.') {
        let cls = &sel[1..];
        return node.classes.iter().any(|c| c == cls);
    }
    // Tag selector
    if let Some(tag) = &node.tag {
        return tag == sel;
    }
    false
}

/// Recursively collect text content from a subtree.
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

/// Recursively free all descendant nodes of a parent, returning their IDs to the free list.
/// Also removes any canvas mappings for freed nodes.
fn free_descendants(st: &mut SharedState, node_id: NodeId) {
    // Collect children first (avoid borrow conflict)
    let children: Vec<NodeId> = st.document.get_node(node_id)
        .map(|n| n.children.clone())
        .unwrap_or_default();

    for child_id in children {
        // Recursively free the child's descendants
        free_descendants(st, child_id);
        // Remove canvas mapping if any
        st.node_canvas_map.remove(&child_id);
        // Add to free list for reuse
        st.document.free_list.push(child_id);
    }
}

/// Set innerHTML on a node: parse HTML fragment, remove old children, add new ones.
fn set_inner_html(st: &mut SharedState, node_id: NodeId, html: &str) {
    // Recursively free all descendant nodes (prevents unbounded node growth)
    free_descendants(st, node_id);

    // Clear the parent's children list
    if let Some(node) = st.document.get_node_mut(node_id) {
        node.children.clear();
        // If the node was a Text node, convert it to Container
        if matches!(node.kind, NodeKind::Text { .. }) {
            node.kind = NodeKind::Container;
        }
    }

    // Parse the HTML fragment and add as children
    if !html.trim().is_empty() {
        add_html_children(st, node_id, html);
    }

    st.layout_dirty = true;
}

/// Parse an HTML fragment and add the resulting nodes as children of parent_id.
fn add_html_children(st: &mut SharedState, parent_id: NodeId, html: &str) {
    // Use a simplified inline HTML parser for fragments
    // This handles basic tags like <div>, <span>, <p>, text nodes, etc.
    let bytes = html.as_bytes();
    let mut pos = 0;

    while pos < bytes.len() {
        if bytes[pos] == b'<' {
            // Skip comments
            if pos + 3 < bytes.len() && &html[pos..pos+4] == "<!--" {
                if let Some(end) = html[pos..].find("-->") {
                    pos += end + 3;
                    continue;
                }
            }

            // Close tag
            if pos + 1 < bytes.len() && bytes[pos + 1] == b'/' {
                // Find end of close tag
                while pos < bytes.len() && bytes[pos] != b'>' { pos += 1; }
                if pos < bytes.len() { pos += 1; }
                return; // Return to parent
            }

            // Open tag
            pos += 1;
            let tag_start = pos;
            while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
                pos += 1;
            }
            let tag = html[tag_start..pos].trim().to_lowercase();

            // Parse attributes
            let mut classes = Vec::new();
            let mut html_id = None;
            let mut inline_style = String::new();
            let mut attributes = HashMap::new();

            loop {
                while pos < bytes.len() && bytes[pos].is_ascii_whitespace() { pos += 1; }
                if pos >= bytes.len() || bytes[pos] == b'>' || bytes[pos] == b'/' { break; }

                let attr_start = pos;
                while pos < bytes.len() && bytes[pos] != b'=' && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' && bytes[pos] != b'/' {
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
                        while pos < bytes.len() && !bytes[pos].is_ascii_whitespace() && bytes[pos] != b'>' { pos += 1; }
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

            // Create node
            let mut node = crate::cxrd::node::CxrdNode::container(0);
            node.tag = Some(tag.clone());
            node.html_id = html_id;
            node.classes = classes;
            node.attributes = attributes;
            node.kind = NodeKind::Container;

            // Apply inline styles
            if !inline_style.is_empty() {
                for decl in inline_style.split(';') {
                    let decl = decl.trim();
                    if let Some((prop, val)) = decl.split_once(':') {
                        apply_property(&mut node.style, prop.trim(), val.trim(), &st.css_variables);
                    }
                }
            }

            // Apply matching CSS rules
            for rule in &st.css_rules {
                if simple_rule_matches(&rule.selector, &node) {
                    for (prop, val) in &rule.declarations {
                        apply_property(&mut node.style, prop, val, &st.css_variables);
                    }
                }
            }

            // Inherit parent style
            if let Some(parent) = st.document.get_node(parent_id) {
                node.style.color = parent.style.color;
                node.style.font_size = parent.style.font_size;
                node.style.font_family = parent.style.font_family.clone();
                node.style.font_weight = parent.style.font_weight;
                node.style.letter_spacing = parent.style.letter_spacing;
                node.style.line_height = parent.style.line_height;
            }

            let child_id = st.document.add_node(node);
            st.document.add_child(parent_id, child_id);

            // For canvas elements, create a canvas buffer (same as __cx_createElement)
            if tag == "canvas" {
                let cid = st.canvas_manager.create_canvas(300, 150);
                st.node_canvas_map.insert(child_id, cid);
            }

            // Parse children for non-void, non-self-closing tags
            let void_tags = ["img", "br", "hr", "input", "meta", "link", "source"];
            if !self_closing && !void_tags.contains(&tag.as_str()) {
                add_html_children(st, child_id, &html[pos..]);
                // Skip past the close tag
                let close_tag = format!("</{}>", tag);
                if let Some(close_pos) = html[pos..].to_lowercase().find(&close_tag) {
                    pos += close_pos + close_tag.len();
                }
            }
        } else {
            // Text content
            let text_start = pos;
            while pos < bytes.len() && bytes[pos] != b'<' { pos += 1; }
            let text = html[text_start..pos].trim();
            if !text.is_empty() {
                let text_node = crate::cxrd::node::CxrdNode::text(0, text);
                let text_id = st.document.add_node(text_node);
                st.document.add_child(parent_id, text_id);
            }
        }
    }
}

/// Simple CSS rule matching for dynamically created nodes.
fn simple_rule_matches(selector: &str, node: &crate::cxrd::node::CxrdNode) -> bool {
    let parts: Vec<&str> = selector.split_whitespace().collect();
    if parts.is_empty() { return false; }
    let last = parts.last().unwrap();

    // Universal selector matches everything.
    if *last == "*" {
        return true;
    }

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

/// Read a computed style value as a string (for getComputedStyle).
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
        "font-size" | "fontSize" => format!("{}px", style.font_size),
        "color" => format!("rgba({},{},{},{})",
            (style.color.r * 255.0) as u8,
            (style.color.g * 255.0) as u8,
            (style.color.b * 255.0) as u8,
            style.color.a,
        ),
        "width" => {
            match style.width {
                crate::cxrd::value::Dimension::Px(v) => format!("{}px", v),
                _ => "auto".into(),
            }
        },
        "height" => {
            match style.height {
                crate::cxrd::value::Dimension::Px(v) => format!("{}px", v),
                _ => "auto".into(),
            }
        },
        _ => String::new(),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// JavaScript DOM/Canvas2D Shim
// ═══════════════════════════════════════════════════════════════════════════

/// The JavaScript shim that wraps native __cx_* functions into standard web APIs.
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
        get innerHTML() { return ''; }, // Reading innerHTML not fully supported
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
            // Search within this element's subtree
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
                        return __cx_getStyle(nid, p);
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
            var sz = __cx_getNodeClientSize(nid).split(',');
            return { x: 0, y: 0, width: parseInt(sz[0]) || 0, height: parseInt(sz[1]) || 0, top: 0, left: 0, bottom: parseInt(sz[1]) || 0, right: parseInt(sz[0]) || 0 };
        },
        // ─── DOM manipulation ───
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
    };
    __cx_elementCache[nid] = el;
    return el;
}

function isDescendant(parentNid, childNid) {
    // Simple check — accept all for now
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

    var ctx = {
        get canvas() {
            // Find the element that owns this canvas
            for (var nid in __cx_elementCache) {
                if (__cx_getCanvasId(parseInt(nid)) === cid) return __cx_elementCache[nid];
            }
            return null;
        },
        // ─── Style properties ───
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
        set textAlign(v) { _textAlign = v; },
        get textBaseline() { return _textBaseline; },
        set textBaseline(v) { _textBaseline = v; },
        get lineCap() { return _lineCap; },
        set lineCap(v) { _lineCap = v; },
        get lineJoin() { return _lineJoin; },
        set lineJoin(v) { _lineJoin = v; },

        // ─── Drawing methods ───
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
        strokeText: function() {}, // stub
        measureText: function(text) { return { width: text.length * 7 }; }, // rough estimate
        setLineDash: function(segments) {}, // stub — CanvasX does not yet support dashed lines
        getLineDash: function() { return []; }, // stub
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
window.innerWidth = 1920;
window.innerHeight = 1080;

// ─── Global event listeners (stubs — CanvasX handles resize internally) ───
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

// ─── setTimeout / setInterval (simplified) ───
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
    // Process timeouts
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

    // Process intervals
    for (var k = 0; k < __cx_intervals.length; k++) {
        if (now >= __cx_intervals[k].triggerAt) {
            try { __cx_intervals[k].callback(); } catch(e) { console.error('Interval error:', e); }
            __cx_intervals[k].triggerAt = now + __cx_intervals[k].interval;
        }
    }

    // Process rAF callbacks (snapshot the list, then clear)
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

// ─── JSON (ensure it exists) ───
if (typeof JSON === 'undefined') {
    // boa_engine provides JSON by default, but just in case
}

// ─── Math extensions ───
if (typeof Math.clamp === 'undefined') {
    Math.clamp = function(v, min, max) { return Math.min(Math.max(v, min), max); };
}

// ─── Array.from polyfill ───
if (typeof Array.from === 'undefined') {
    Array.from = function(arr) {
        var result = [];
        for (var i = 0; i < arr.length; i++) result.push(arr[i]);
        return result;
    };
}

// Viewport size update
try {
    var __vp = __cx_getViewportSize().split(',');
    window.innerWidth = parseInt(__vp[0]) || 1920;
    window.innerHeight = parseInt(__vp[1]) || 1080;
} catch(e) {}
"#;
