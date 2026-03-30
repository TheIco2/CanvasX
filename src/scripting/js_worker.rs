// openrender-runtime/src/scripting/js_worker.rs
//
// Offloads JavaScript execution to a dedicated background thread.
// The render thread stays free to present frames at sub-millisecond latency
//
// Communication:
//   Main → JS:  JsCommand  (tick, restyle, data updates, script exec, shutdown)
//   JS → Main:  JsResult   (dirty canvas pixels, document snapshots, flags)

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc;
use std::thread::{self, JoinHandle};

use crate::cxrd::document::CxrdDocument;
use crate::cxrd::node::NodeId;
use crate::compiler::css::CssRule;
use crate::compiler::html::ScriptBlock;
use crate::scripting::canvas2d::CanvasId;
use crate::scripting::v8_runtime::JsRuntime;

// ── Messages ──────────────────────────────────────────────────────────────

/// Commands sent from the main/render thread to the JS worker.
pub enum JsCommand {
    /// Run one rAF tick.
    Tick(f32),
    /// Re-apply CSS rules and return the updated document.
    Restyle,
    /// Bulk-update data values (IPC system data → JS OpenDesktop.subscribe).
    UpdateData(HashMap<String, String>),
    /// Stop the worker.
    Shutdown,
}

/// Canvas pixel data produced by a JS tick.
pub struct DirtyCanvas {
    pub canvas_id: CanvasId,
    pub node_id: Option<NodeId>,
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

/// Results sent from the JS worker back to the main thread.
pub enum JsResult {
    /// Initial setup complete (scripts executed, first restyle done).
    InitDone {
        document: CxrdDocument,
        uses_data_tags: bool,
    },
    /// A tick completed. Contains dirty canvas data + flags.
    TickDone {
        dirty_canvases: Vec<DirtyCanvas>,
        layout_dirty: bool,
        node_canvas_map: HashMap<NodeId, CanvasId>,
    },
    /// Restyle completed. Contains the updated document snapshot.
    RestyleDone {
        document: CxrdDocument,
    },
}

// ── Configuration ─────────────────────────────────────────────────────────

/// Everything needed to initialise the JS runtime inside the worker thread.
pub struct JsWorkerInit {
    pub document: CxrdDocument,
    pub css_rules: Vec<CssRule>,
    pub css_variables: HashMap<String, String>,
    pub scripts: Vec<ScriptBlock>,
    pub asset_dir: PathBuf,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

// ── Handle ────────────────────────────────────────────────────────────────

/// Handle held by the main thread to communicate with the JS worker.
pub struct JsWorkerHandle {
    cmd_tx: mpsc::Sender<JsCommand>,
    result_rx: mpsc::Receiver<JsResult>,
    _thread: JoinHandle<()>,
}

impl JsWorkerHandle {
    /// Spawn the JS worker thread.
    pub fn spawn(init: JsWorkerInit) -> Self {
        let (cmd_tx, cmd_rx) = mpsc::channel::<JsCommand>();
        let (result_tx, result_rx) = mpsc::channel::<JsResult>();

        let handle = thread::Builder::new()
            .name("js-worker".into())
            .spawn(move || {
                js_worker_main(cmd_rx, result_tx, init);
            })
            .expect("Failed to spawn JS worker thread");

        Self {
            cmd_tx,
            result_rx,
            _thread: handle,
        }
    }

    /// Send a tick command (non-blocking). Drops the command if the
    /// channel is full or disconnected.
    pub fn send_tick(&self, dt: f32) {
        let _ = self.cmd_tx.send(JsCommand::Tick(dt));
    }

    /// Send a restyle command.
    pub fn send_restyle(&self) {
        let _ = self.cmd_tx.send(JsCommand::Restyle);
    }

    /// Send updated IPC data values to the JS thread.
    pub fn send_data(&self, data: HashMap<String, String>) {
        let _ = self.cmd_tx.send(JsCommand::UpdateData(data));
    }

    /// Request graceful shutdown.
    pub fn shutdown(&self) {
        let _ = self.cmd_tx.send(JsCommand::Shutdown);
    }

    /// Poll for results (non-blocking). Returns all available results.
    pub fn poll_results(&self) -> Vec<JsResult> {
        let mut results = Vec::new();
        while let Ok(r) = self.result_rx.try_recv() {
            results.push(r);
        }
        results
    }

    /// Block until the worker finishes initial setup and returns the
    /// document + uses_data_tags flag. Must be called exactly once,
    /// immediately after `spawn()`. Returns `None` if the worker
    /// died or sent an unexpected result.
    pub fn wait_for_init(&self) -> Option<(CxrdDocument, bool)> {
        match self.result_rx.recv_timeout(std::time::Duration::from_secs(15)) {
            Ok(JsResult::InitDone { document, uses_data_tags }) => Some((document, uses_data_tags)),
            Ok(_other) => {
                log::error!("[JS-WORKER] Expected InitDone, got unexpected result");
                None
            }
            Err(e) => {
                log::error!("[JS-WORKER] wait_for_init failed: {}", e);
                None
            }
        }
    }
}

// ── Worker thread ─────────────────────────────────────────────────────────

fn js_worker_main(
    cmd_rx: mpsc::Receiver<JsCommand>,
    result_tx: mpsc::Sender<JsResult>,
    init: JsWorkerInit,
) {
    log::info!("[JS-WORKER] Thread started, initialising runtime...");

    let mut js_rt = JsRuntime::new(
        init.document,
        init.css_rules,
        init.css_variables,
    );

    js_rt.init_canvases(init.viewport_width, init.viewport_height);

    // Execute scripts (opendesktop.js, user scripts, etc.)
    for script in &init.scripts {
        if let Some(ref src) = script.src {
            let script_path = init.asset_dir.join(src);
            log::warn!("[JS-WORKER] Loading script: {}", script_path.display());
            js_rt.execute_file(&script_path);
        } else if !script.content.is_empty() {
            log::warn!("[JS-WORKER] Executing inline script ({} bytes)", script.content.len());
            js_rt.execute(&script.content, "<inline>");
        }
    }

    // Cache rAF function reference for direct calls
    js_rt.cache_raf_tick_fn();

    // Initial restyle so the document is fully styled before we return it.
    js_rt.restyle();
    let init_doc = js_rt.document().clone();
    let uses_data_tags = init_doc.nodes.iter().any(|n| {
        matches!(n.tag.as_deref(), Some("data-bind") | Some("data-bar"))
    });
    let _ = result_tx.send(JsResult::InitDone {
        document: init_doc,
        uses_data_tags,
    });

    log::info!("[JS-WORKER] Initialisation complete, entering command loop");

    loop {
        // Block until a command arrives.
        let cmd = match cmd_rx.recv() {
            Ok(c) => c,
            Err(_) => {
                log::info!("[JS-WORKER] Channel closed, shutting down");
                break;
            }
        };

        match cmd {
            JsCommand::Tick(dt) => {
                // Drain any stale tick commands — only execute the latest.
                let mut latest_dt = dt;
                while let Ok(next) = cmd_rx.try_recv() {
                    match next {
                        JsCommand::Tick(d) => latest_dt = d,
                        JsCommand::Shutdown => {
                            log::info!("[JS-WORKER] Shutdown during tick drain");
                            return;
                        }
                        JsCommand::Restyle => {
                            // Process restyle inline
                            js_rt.restyle();
                            let doc = js_rt.document().clone();
                            let _ = result_tx.send(JsResult::RestyleDone { document: doc });
                        }
                        JsCommand::UpdateData(data) => {
                            let mut st = js_rt.state.borrow_mut();
                            st.data_values.extend(data);
                        }
                    }
                }

                js_rt.gc_gradients();
                let _canvas_dirty = js_rt.tick(latest_dt);
                let layout_dirty = js_rt.take_layout_dirty();

                // Collect dirty canvases
                let dirty_canvases: Vec<DirtyCanvas> = js_rt.dirty_canvases()
                    .into_iter()
                    .map(|(cid, nid, w, h, px)| DirtyCanvas {
                        canvas_id: cid,
                        node_id: nid,
                        width: w,
                        height: h,
                        pixels: px,
                    })
                    .collect();
                js_rt.clear_dirty_flags();

                // Snapshot the node→canvas map
                let ncm = js_rt.state.borrow().node_canvas_map.clone();

                let _ = result_tx.send(JsResult::TickDone {
                    dirty_canvases,
                    layout_dirty,
                    node_canvas_map: ncm,
                });
            }

            JsCommand::Restyle => {
                js_rt.restyle();
                let doc = js_rt.document().clone();
                let _ = result_tx.send(JsResult::RestyleDone { document: doc });
            }

            JsCommand::UpdateData(data) => {
                let mut st = js_rt.state.borrow_mut();
                st.data_values.extend(data);
            }

            JsCommand::Shutdown => {
                log::info!("[JS-WORKER] Shutdown requested");
                break;
            }
        }
    }
}
