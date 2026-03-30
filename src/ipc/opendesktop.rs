// OpenDesktop-runtime/src/ipc/OpenDesktop.rs
//
// OpenDesktop IPC Bridge — high-level integration module for connecting
// OpenDesktop scenes to the OpenDesktop backend.
//
// This module speaks OpenDesktop's exact protocol format (`{ns, cmd, args}`
// over `\\.\pipe\OpenDesktop`) and provides convenient Rust APIs for:
//   - Fetching system data (CPU, GPU, RAM, etc.)
//   - Reading the addon/asset registry
//   - Controlling the backend (polling rates, tracking demands)
//   - Sending heartbeats
//
// This module is optional — OpenDesktop works standalone without it.
// It is enabled when the host application is OpenDesktop.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, atomic::{AtomicBool, AtomicU64, Ordering}};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use crate::ipc::client::{send_ipc_request_to, flatten_json_to_map};
use crate::ipc::protocol::{IpcRequest, IpcResponse};

/// The OpenDesktop pipe name.
pub const OPEN_DESKTOP_PIPE: &str = r"\\.\pipe\opendesktop";

/// A high-level OpenDesktop IPC client.
///
/// Connects to OpenDesktop-core's named pipe server and provides structured
/// access to system data, registry, and backend controls.
pub struct OpenDesktopBridge {
    /// The pipe name to connect to.
    pipe_name: String,

    /// Latest system data snapshot (flat key-value).
    pub sysdata: Arc<Mutex<HashMap<String, String>>>,

    /// Latest raw sysdata JSON (for forwarding full categories).
    pub sysdata_raw: Arc<Mutex<Value>>,

    /// Latest appdata JSON (per-monitor foreground/active window info).
    pub appdata_raw: Arc<Mutex<Value>>,

    /// Whether the bridge is connected and polling.
    pub connected: Arc<AtomicBool>,

    /// Tracking demands — which data sections to request.
    tracking_demands: Arc<Mutex<Vec<String>>>,

    /// Polling interval in ms.
    poll_interval_ms: Arc<AtomicU64>,

    /// Whether polling is active.
    polling_active: Arc<AtomicBool>,

    /// Shutdown flag for the background thread.
    shutdown: Arc<AtomicBool>,

    /// Background thread handle.
    _thread: Option<thread::JoinHandle<()>>,
}

/// Configuration for the OpenDesktop bridge.
#[derive(Clone)]
pub struct OpenDesktopBridgeConfig {
    /// Pipe name (default: `\\.\pipe\opendesktop`).
    pub pipe_name: String,
    /// Which data sections to track (e.g., "cpu", "gpu", "ram", "time", etc.).
    pub tracking_demands: Vec<String>,
    /// Polling interval in ms (default: 50, matching OpenDesktop's fast pull).
    pub poll_interval_ms: u64,
    /// Whether to send UI heartbeats (keeps backend data alive).
    pub send_heartbeats: bool,
}

impl Default for OpenDesktopBridgeConfig {
    fn default() -> Self {
        Self {
            pipe_name: OPEN_DESKTOP_PIPE.into(),
            tracking_demands: vec![
                "time", "cpu", "gpu", "ram", "storage", "displays",
                "network", "wifi", "bluetooth", "audio", "keyboard",
                "mouse", "power", "idle", "system", "media",
            ].into_iter().map(String::from).collect(),
            poll_interval_ms: 250,
            send_heartbeats: true,
        }
    }
}

impl OpenDesktopBridge {
    /// Create and start a new OpenDesktop bridge with default config.
    pub fn start() -> Self {
        Self::with_config(OpenDesktopBridgeConfig::default())
    }

    /// Create and start with custom config.
    pub fn with_config(config: OpenDesktopBridgeConfig) -> Self {
        let sysdata = Arc::new(Mutex::new(HashMap::new()));
        let sysdata_raw = Arc::new(Mutex::new(Value::Null));
        let appdata_raw = Arc::new(Mutex::new(Value::Null));
        let connected = Arc::new(AtomicBool::new(false));
        let tracking_demands = Arc::new(Mutex::new(config.tracking_demands.clone()));
        let poll_interval_ms = Arc::new(AtomicU64::new(config.poll_interval_ms));
        let polling_active = Arc::new(AtomicBool::new(true));
        let shutdown = Arc::new(AtomicBool::new(false));

        let thread_state = BridgeThreadState {
            pipe_name: config.pipe_name.clone(),
            sysdata: sysdata.clone(),
            sysdata_raw: sysdata_raw.clone(),
            appdata_raw: appdata_raw.clone(),
            connected: connected.clone(),
            tracking_demands: tracking_demands.clone(),
            poll_interval_ms: poll_interval_ms.clone(),
            polling_active: polling_active.clone(),
            shutdown: shutdown.clone(),
            send_heartbeats: config.send_heartbeats,
        };

        let handle = thread::spawn(move || {
            opendesktop_poll_loop(thread_state);
        });

        Self {
            pipe_name: config.pipe_name,
            sysdata,
            sysdata_raw,
            appdata_raw,
            connected,
            tracking_demands,
            poll_interval_ms,
            polling_active,
            shutdown,
            _thread: Some(handle),
        }
    }

    /// Check if the bridge is connected to OpenDesktop-core.
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Relaxed)
    }

    /// Get flattened system data as key-value pairs.
    /// Keys are dotted paths like "cpu.usage", "ram.used_gb", etc.
    pub fn get_data(&self) -> HashMap<String, String> {
        self.sysdata.lock().unwrap().clone()
    }

    /// Get a single data value.
    pub fn get_value(&self, key: &str) -> Option<String> {
        self.sysdata.lock().unwrap().get(key).cloned()
    }

    /// Get the raw sysdata JSON (all categories).
    pub fn get_sysdata_json(&self) -> Value {
        self.sysdata_raw.lock().unwrap().clone()
    }

    /// Get the raw appdata JSON.
    pub fn get_appdata_json(&self) -> Value {
        self.appdata_raw.lock().unwrap().clone()
    }

    /// Set which data sections to track.
    pub fn set_tracking_demands(&self, sections: Vec<String>) {
        *self.tracking_demands.lock().unwrap() = sections;
    }

    /// Set polling interval (in ms).
    pub fn set_poll_interval(&self, ms: u64) {
        self.poll_interval_ms.store(ms, Ordering::Relaxed);
    }

    /// Pause data polling.
    pub fn pause(&self) {
        self.polling_active.store(false, Ordering::Relaxed);
    }

    /// Resume data polling.
    pub fn resume(&self) {
        self.polling_active.store(true, Ordering::Relaxed);
    }

    // --- One-shot IPC commands ---

    /// Send a raw IPC request to OpenDesktop-core.
    pub fn send_request(&self, ns: &str, cmd: &str, args: Option<Value>) -> Result<IpcResponse, String> {
        let request = match args {
            Some(a) => IpcRequest::with_args(ns, cmd, a),
            None => IpcRequest::new(ns, cmd),
        };
        send_ipc_request_to(&self.pipe_name, request)
    }

    /// Get the list of registered addons.
    pub fn list_addons(&self) -> Result<Value, String> {
        self.send_request("registry", "list_addons", None)
            .and_then(|r| r.data.ok_or_else(|| "No data".into()))
    }

    /// Get the list of registered assets.
    pub fn list_assets(&self) -> Result<Value, String> {
        self.send_request("registry", "list_assets", None)
            .and_then(|r| r.data.ok_or_else(|| "No data".into()))
    }

    /// Start an addon by name.
    pub fn start_addon(&self, addon_name: &str) -> Result<IpcResponse, String> {
        self.send_request("addon", "start", Some(json!({ "addon_name": addon_name })))
    }

    /// Stop an addon by name.
    pub fn stop_addon(&self, addon_name: &str) -> Result<IpcResponse, String> {
        self.send_request("addon", "stop", Some(json!({ "addon_name": addon_name })))
    }

    /// Reload an addon.
    pub fn reload_addon(&self, addon_name: &str) -> Result<IpcResponse, String> {
        self.send_request("addon", "reload", Some(json!({ "addon_name": addon_name })))
    }

    /// Get the backend config.
    pub fn get_backend_config(&self) -> Result<Value, String> {
        self.send_request("backend", "get_config", None)
            .and_then(|r| r.data.ok_or_else(|| "No data".into()))
    }

    /// Set the fast pull rate.
    pub fn set_fast_pull_rate(&self, rate_ms: u64) -> Result<IpcResponse, String> {
        self.send_request("backend", "set_fast_pull_rate", Some(json!({ "rate_ms": rate_ms })))
    }

    /// Set the slow pull rate.
    pub fn set_slow_pull_rate(&self, rate_ms: u64) -> Result<IpcResponse, String> {
        self.send_request("backend", "set_slow_pull_rate", Some(json!({ "rate_ms": rate_ms })))
    }

    /// Set data pull paused state.
    pub fn set_pull_paused(&self, paused: bool) -> Result<IpcResponse, String> {
        self.send_request("backend", "set_pull_paused", Some(json!({ "paused": paused })))
    }

    /// Send a UI heartbeat (tells OpenDesktop-core the UI is active).
    pub fn ui_heartbeat(&self) -> Result<IpcResponse, String> {
        self.send_request("backend", "ui_heartbeat", None)
    }

    /// Full registry snapshot (addons + assets + sysdata + appdata).
    pub fn full_snapshot(&self) -> Result<Value, String> {
        self.send_request("registry", "full", None)
            .and_then(|r| r.data.ok_or_else(|| "No data".into()))
    }
}

// --- Background thread ---

struct BridgeThreadState {
    pipe_name: String,
    sysdata: Arc<Mutex<HashMap<String, String>>>,
    sysdata_raw: Arc<Mutex<Value>>,
    appdata_raw: Arc<Mutex<Value>>,
    connected: Arc<AtomicBool>,
    tracking_demands: Arc<Mutex<Vec<String>>>,
    poll_interval_ms: Arc<AtomicU64>,
    polling_active: Arc<AtomicBool>,
    shutdown: Arc<AtomicBool>,
    send_heartbeats: bool,
}

fn opendesktop_poll_loop(state: BridgeThreadState) {
    log::info!("OpenDesktop bridge: starting (pipe: {})", state.pipe_name);

    let mut heartbeat_timer = Instant::now();
    let mut last_demands_sent: Option<Vec<String>> = None;

    while !state.shutdown.load(Ordering::Relaxed) {
        let interval = state.poll_interval_ms.load(Ordering::Relaxed);

        if !state.polling_active.load(Ordering::Relaxed) {
            thread::sleep(Duration::from_millis(100));
            continue;
        }

        // Send tracking demands whenever they change (including clear []).
        let sections = state.tracking_demands.lock().unwrap().clone();
        if last_demands_sent.as_ref() != Some(&sections) {
            let request = IpcRequest::with_args(
                "backend",
                "set_tracking_demands",
                json!({ "sections": sections }),
            );
            if send_ipc_request_to(&state.pipe_name, request).is_ok() {
                last_demands_sent = Some(state.tracking_demands.lock().unwrap().clone());
            }
        }

        // Fetch snapshot (synchronous — one connection at a time).
        let sections = state.tracking_demands.lock().unwrap().clone();
        let request = IpcRequest::with_args(
            "registry",
            "snapshot",
            json!({ "sections": sections }),
        );

        match send_ipc_request_to(&state.pipe_name, request) {
            Ok(resp) if resp.ok => {
                if !state.connected.load(Ordering::Relaxed) {
                    log::info!("OpenDesktop bridge: connected");
                }
                state.connected.store(true, Ordering::Relaxed);

                if let Some(data) = resp.data {
                    // Extract sysdata.
                    if let Some(sysdata) = data.get("sysdata") {
                        *state.sysdata_raw.lock().unwrap() = sysdata.clone();
                        flatten_json_to_map(sysdata, "", &mut state.sysdata.lock().unwrap());
                    }

                    // Extract appdata.
                    if let Some(appdata) = data.get("appdata") {
                        *state.appdata_raw.lock().unwrap() = appdata.clone();
                    }
                }
            }
            Ok(_) => {
                state.connected.store(true, Ordering::Relaxed);
            }
            Err(e) => {
                if state.connected.load(Ordering::Relaxed) {
                    log::warn!("OpenDesktop bridge: connection lost ({})", e);
                }
                state.connected.store(false, Ordering::Relaxed);
                // Re-send demands on reconnect.
                last_demands_sent = None;
                thread::sleep(Duration::from_millis(1000));
                continue;
            }
        }

        // Heartbeat every 500ms.
        if state.send_heartbeats && heartbeat_timer.elapsed() >= Duration::from_millis(500) {
            let _ = send_ipc_request_to(
                &state.pipe_name,
                IpcRequest::new("backend", "ui_heartbeat"),
            );
            heartbeat_timer = Instant::now();
        }

        thread::sleep(Duration::from_millis(interval));
    }

    log::info!("OpenDesktop bridge: stopped");
}

impl Drop for OpenDesktopBridge {
    fn drop(&mut self) {
        // Stop polling loop and request background thread exit.
        self.shutdown.store(true, Ordering::Relaxed);
        self.polling_active.store(false, Ordering::Relaxed);

        // Best-effort clear of explicit tracking demands so OpenDesktop-core can idle.
        let _ = send_ipc_request_to(
            &self.pipe_name,
            IpcRequest::with_args("backend", "set_tracking_demands", json!({ "sections": Vec::<String>::new() })),
        );

        if let Some(handle) = self._thread.take() {
            let _ = handle.join();
        }
    }
}

// flatten_json_to_map is now imported from crate::ipc::client
