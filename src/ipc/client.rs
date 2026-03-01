// canvasx-runtime/src/ipc/client.rs
//
// Generic named-pipe IPC client — connects to any host application that
// implements the CanvasX IPC protocol to exchange data and commands.
//
// The pipe name and polling behaviour are fully configurable.
// No hard-coded dependency on any specific host application.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use windows::core::PCWSTR;
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE, ERROR_PIPE_BUSY},
    Storage::FileSystem::{
        CreateFileW, ReadFile, WriteFile, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
        FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES,
    },
    System::Pipes::{WaitNamedPipeW, SetNamedPipeHandleState, PIPE_READMODE_MESSAGE},
};

use crate::ipc::protocol::{IpcRequest, IpcResponse};

/// Default pipe name — host applications can override this.
pub const DEFAULT_PIPE_NAME: &str = r"\\.\pipe\canvasx";
const READ_CHUNK: usize = 64 * 1024;
const DEFAULT_POLL_INTERVAL_MS: u64 = 250;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

/// Shared data store updated by the IPC background thread.
pub type SharedData = Arc<Mutex<HashMap<String, String>>>;

/// Configuration for the IPC client.
#[derive(Clone)]
pub struct IpcConfig {
    /// Named pipe path (e.g., `\\.\pipe\myapp`).
    pub pipe_name: String,
    /// Polling interval in milliseconds.
    pub poll_interval_ms: u64,
    /// Namespace and command to use for data polling.
    /// Set to `None` to disable automatic polling.
    pub poll_request: Option<(String, String)>,
}

impl Default for IpcConfig {
    fn default() -> Self {
        Self {
            pipe_name: DEFAULT_PIPE_NAME.to_string(),
            poll_interval_ms: DEFAULT_POLL_INTERVAL_MS,
            poll_request: Some(("sysdata".into(), "getAll".into())),
        }
    }
}

/// The IPC client manager.
pub struct IpcClient {
    /// Shared data updated by the background poller.
    pub data: SharedData,
    /// The pipe name this client connects to.
    pub pipe_name: String,
    /// Handle to the background thread.
    _thread: Option<thread::JoinHandle<()>>,
}

impl IpcClient {
    /// Create a new IPC client with default configuration and start polling.
    pub fn start() -> Self {
        Self::with_config(IpcConfig::default())
    }

    /// Create a new IPC client with custom configuration.
    pub fn with_config(config: IpcConfig) -> Self {
        let data: SharedData = Arc::new(Mutex::new(HashMap::new()));
        let data_clone = data.clone();
        let pipe_name = config.pipe_name.clone();

        let handle = thread::spawn(move || {
            ipc_poll_loop(data_clone, config);
        });

        Self {
            data,
            pipe_name,
            _thread: Some(handle),
        }
    }

    /// Get a snapshot of all current data values.
    pub fn snapshot(&self) -> HashMap<String, String> {
        self.data.lock().unwrap().clone()
    }

    /// Send a one-off IPC request (blocking) to the default pipe.
    pub fn send(&self, request: IpcRequest) -> Result<IpcResponse, String> {
        send_ipc_request_to(&self.pipe_name, request)
    }

    /// Send a one-off IPC request to a specific pipe (blocking, static).
    pub fn send_to(pipe_name: &str, request: IpcRequest) -> Result<IpcResponse, String> {
        send_ipc_request_to(pipe_name, request)
    }
}

/// Background polling loop — periodically fetches data from the host.
fn ipc_poll_loop(data: SharedData, config: IpcConfig) {
    log::info!("IPC client: background poller starting (pipe: {})", config.pipe_name);

    let (ns, cmd) = match config.poll_request {
        Some((ns, cmd)) => (ns, cmd),
        None => {
            log::info!("IPC polling disabled — no poll_request configured");
            return;
        }
    };

    loop {
        let request = IpcRequest::new(&ns, &cmd);
        match send_ipc_request_to(&config.pipe_name, request) {
            Ok(resp) if resp.ok => {
                if let Some(serde_json::Value::Object(map)) = resp.data {
                    let mut lock = data.lock().unwrap();
                    for (key, val) in map {
                        let val_str = match &val {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Number(n) => n.to_string(),
                            serde_json::Value::Bool(b) => b.to_string(),
                            other => other.to_string(),
                        };
                        lock.insert(key, val_str);
                    }
                }
            }
            Ok(resp) => {
                if let Some(err) = resp.error {
                    log::debug!("IPC poll error: {}", err);
                }
            }
            Err(e) => {
                log::debug!("IPC connection error: {} (will retry)", e);
            }
        }

        thread::sleep(Duration::from_millis(config.poll_interval_ms));
    }
}

/// Send an IPC request over a named pipe (blocking).
/// Public so that other modules (e.g. `ipc::sentinel`) can call it directly.
pub fn send_ipc_request_to(pipe_name: &str, request: IpcRequest) -> Result<IpcResponse, String> {
    unsafe {
        let wide_name = to_wide(pipe_name);

        let handle: HANDLE = {
            let mut attempts = 0;
            loop {
                let result = CreateFileW(
                    PCWSTR(wide_name.as_ptr()),
                    FILE_GENERIC_READ.0 | FILE_GENERIC_WRITE.0,
                    FILE_SHARE_READ | FILE_SHARE_WRITE,
                    None,
                    OPEN_EXISTING,
                    FILE_FLAGS_AND_ATTRIBUTES(0),
                    None,
                );

                match result {
                    Ok(h) => break h,
                    Err(err) => {
                        let code = err.code().0 as u32;
                        if code == ERROR_PIPE_BUSY.0 {
                            attempts += 1;
                            if attempts > 3 {
                                return Err("IPC pipe busy after 3 retries".into());
                            }
                            let _ = WaitNamedPipeW(PCWSTR(wide_name.as_ptr()), 2000);
                            continue;
                        }
                        return Err(format!("IPC connect failed: {:?}", err));
                    }
                }
            }
        };

        // Switch to message-read mode.
        let mut mode = PIPE_READMODE_MESSAGE;
        let _ = SetNamedPipeHandleState(handle, Some(&mut mode), None, None);

        // Write request.
        let payload = serde_json::to_vec(&request)
            .map_err(|e| format!("Serialize error: {}", e))?;
        let mut written = 0u32;
        if WriteFile(handle, Some(&payload), Some(&mut written), None).is_err() {
            let _ = CloseHandle(handle);
            return Err("IPC write failed".into());
        }

        // Read response.
        let mut response = Vec::new();
        let mut chunk = vec![0u8; READ_CHUNK];
        let mut read = 0u32;
        match ReadFile(handle, Some(&mut chunk), Some(&mut read), None) {
            Ok(_) => {
                response.extend_from_slice(&chunk[..read as usize]);
            }
            Err(_) => {
                let _ = CloseHandle(handle);
                return Err("IPC read failed".into());
            }
        }

        let _ = CloseHandle(handle);

        serde_json::from_slice(&response)
            .map_err(|e| format!("Response parse error: {}", e))
    }
}

/// Send an IPC request with a timeout. Spawns a worker thread and waits
/// up to `timeout` for a response. Returns `Err` if the timeout expires.
pub fn send_ipc_request_with_timeout(
    pipe_name: &str,
    request: IpcRequest,
    timeout: Duration,
) -> Result<IpcResponse, String> {
    let pipe_name = pipe_name.to_string();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let result = send_ipc_request_to(&pipe_name, request);
        let _ = tx.send(result);
    });
    rx.recv_timeout(timeout)
        .map_err(|_| "IPC request timed out".to_string())?
}
