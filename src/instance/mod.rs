// openrender-runtime/src/instance/mod.rs
//
// Single-instance enforcement and multi-instance coordination.
//
// When an application declares the `SingleInstance` capability, the runtime
// creates a named mutex to detect duplicate launches and a named pipe to
// receive "focus" signals from subsequent launches.
//
// Platform: Windows  (uses Win32 Mutex + Named Pipes)

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;

use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::System::Threading::{CreateMutexW, OpenMutexW, ReleaseMutex, MUTEX_ALL_ACCESS};

const PIPE_BUFFER_SIZE: u32 = 512;

// Named pipe open-mode flags (raw values, to avoid import path issues).
const PIPE_ACCESS_INBOUND: u32 = 0x0000_0001;
// Named pipe type/readmode/wait flags.
const PIPE_TYPE_MESSAGE: u32 = 0x0000_0004;
const PIPE_READMODE_MESSAGE: u32 = 0x0000_0002;
const PIPE_WAIT: u32 = 0x0000_0000;

/// Result of attempting to acquire the single-instance lock.
pub enum InstanceLockResult {
    /// This is the first (primary) instance.  The guard MUST be kept alive
    /// for the entire application lifetime; dropping it releases the lock.
    Acquired(InstanceGuard),
    /// Another instance is already running.  A focus signal has already been
    /// sent to it.  The caller should exit.
    AlreadyRunning,
}

/// Holds the named mutex and pipe listener for the primary instance.
/// Dropping this releases the mutex.
pub struct InstanceGuard {
    _mutex_handle: HANDLE,
    /// Signals the pipe-listener thread to stop.
    shutdown: Arc<AtomicBool>,
    /// Receives "focus" requests from secondary launches.
    focus_rx: std::sync::mpsc::Receiver<()>,
}

impl InstanceGuard {
    /// Poll for pending focus-request signals (non-blocking).
    pub fn poll_focus_requests(&self) -> Vec<()> {
        let mut requests = Vec::new();
        while let Ok(()) = self.focus_rx.try_recv() {
            requests.push(());
        }
        requests
    }
}

impl Drop for InstanceGuard {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::SeqCst);
        unsafe {
            let _ = ReleaseMutex(self._mutex_handle);
            let _ = CloseHandle(self._mutex_handle);
        }
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Attempt to acquire the single-instance lock for `app_name`.
///
/// - If no other instance holds the lock, returns `Acquired(guard)`.
///   The guard starts a background thread that listens for focus signals
///   from future launches.
///
/// - If an existing instance already holds the lock, sends it a "focus"
///   signal and returns `AlreadyRunning`.
pub fn acquire_single_instance(app_name: &str) -> InstanceLockResult {
    let mutex_name = format!("OpenRender_SingleInstance_{app_name}");
    let pipe_name = format!(r"\\.\pipe\OpenRender_{app_name}");

    let mutex_hs = HSTRING::from(&mutex_name);

    // Probe: try to open a pre-existing mutex.  If it exists, another
    // instance already holds the lock.
    let existing = unsafe { OpenMutexW(MUTEX_ALL_ACCESS, false, &mutex_hs) };
    if let Ok(h) = existing {
        // Mutex already exists — another instance is running.
        unsafe { let _ = CloseHandle(h); }
        log::info!("Detected existing instance via mutex '{mutex_name}'");
        signal_existing_instance(&pipe_name);
        return InstanceLockResult::AlreadyRunning;
    }

    // No existing mutex — we are the primary instance.  Create the mutex.
    let mutex_handle = unsafe {
        CreateMutexW(None, true, &mutex_hs)
    };

    let mutex_handle = match mutex_handle {
        Ok(h) => h,
        Err(e) => {
            log::error!("CreateMutexW failed: {e}");
            return InstanceLockResult::AlreadyRunning;
        }
    };

    // We are the primary instance.  Start the pipe listener.
    let (focus_tx, focus_rx) = std::sync::mpsc::channel();
    let shutdown = Arc::new(AtomicBool::new(false));

    let listener_shutdown = shutdown.clone();
    let listener_pipe = pipe_name.clone();
    thread::Builder::new()
        .name("openrender-instance-listener".into())
        .spawn(move || {
            pipe_listener_thread(&listener_pipe, focus_tx, listener_shutdown);
        })
        .expect("Failed to spawn instance listener thread");

    log::info!("Single-instance lock acquired for '{app_name}'");

    InstanceLockResult::Acquired(InstanceGuard {
        _mutex_handle: mutex_handle,
        shutdown,
        focus_rx,
    })
}

// ---------------------------------------------------------------------------
// Named-pipe listener (runs in background thread on primary instance)
// ---------------------------------------------------------------------------

fn pipe_listener_thread(
    pipe_name: &str,
    focus_tx: std::sync::mpsc::Sender<()>,
    shutdown: Arc<AtomicBool>,
) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::ReadFile;
    use windows::Win32::System::Pipes::{
        ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, NAMED_PIPE_MODE,
    };

    let pipe_hs = HSTRING::from(pipe_name);
    let open_mode = windows::Win32::Storage::FileSystem::FILE_FLAGS_AND_ATTRIBUTES(PIPE_ACCESS_INBOUND);
    let pipe_mode = NAMED_PIPE_MODE(PIPE_TYPE_MESSAGE | PIPE_READMODE_MESSAGE | PIPE_WAIT);

    loop {
        if shutdown.load(Ordering::SeqCst) {
            break;
        }

        // Create a new pipe instance for each connection.
        // CreateNamedPipeW returns HANDLE directly (not Result).
        let pipe = unsafe {
            CreateNamedPipeW(
                &pipe_hs,
                open_mode,
                pipe_mode,
                1,                // max instances
                PIPE_BUFFER_SIZE, // out buffer
                PIPE_BUFFER_SIZE, // in buffer
                0,                // default timeout
                None,             // default security
            )
        };

        if pipe == INVALID_HANDLE_VALUE {
            if !shutdown.load(Ordering::SeqCst) {
                log::error!("CreateNamedPipeW returned INVALID_HANDLE_VALUE");
            }
            break;
        }

        // Block until a client connects.
        let connected = unsafe { ConnectNamedPipe(pipe, None) };
        if shutdown.load(Ordering::SeqCst) {
            unsafe { let _ = CloseHandle(pipe); }
            break;
        }

        if connected.is_err() {
            let err = unsafe { GetLastError() };
            if err != windows::Win32::Foundation::ERROR_PIPE_CONNECTED {
                log::warn!("ConnectNamedPipe error: {err:?}");
                unsafe { let _ = CloseHandle(pipe); }
                continue;
            }
        }

        // Read the message (expecting "FOCUS\n").
        let mut buf = [0u8; 64];
        let mut bytes_read = 0u32;
        let _ = unsafe {
            ReadFile(pipe, Some(&mut buf), Some(&mut bytes_read), None)
        };

        let msg = std::str::from_utf8(&buf[..bytes_read as usize]).unwrap_or("");
        if msg.trim() == "FOCUS" {
            let _ = focus_tx.send(());
            log::info!("Received FOCUS signal from secondary instance");
        }

        unsafe {
            let _ = DisconnectNamedPipe(pipe);
            let _ = CloseHandle(pipe);
        }
    }
}

// ---------------------------------------------------------------------------
// Signal the existing (primary) instance to come to foreground
// ---------------------------------------------------------------------------

fn signal_existing_instance(pipe_name: &str) {
    use windows::Win32::Foundation::CloseHandle;
    use windows::Win32::Storage::FileSystem::{
        CreateFileW, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_NONE, OPEN_EXISTING,
        FILE_GENERIC_WRITE,
    };

    let pipe_hs = HSTRING::from(pipe_name);

    let handle = unsafe {
        CreateFileW(
            &pipe_hs,
            FILE_GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        )
    };

    let handle = match handle {
        Ok(h) => h,
        Err(e) => {
            log::warn!("Could not connect to existing instance pipe: {e}");
            return;
        }
    };

    let msg = b"FOCUS\n";
    let mut written = 0u32;
    let _ = unsafe { WriteFile(handle, Some(msg), Some(&mut written), None) };
    unsafe { let _ = CloseHandle(handle); }

    log::info!("Sent FOCUS signal to existing instance");
}
