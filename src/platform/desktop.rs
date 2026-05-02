// prism-runtime/src/platform/desktop.rs
//
// WorkerW desktop window embedding for wallpapers.
// This finds the desktop's WorkerW layer and creates a child window that
// renders behind desktop icons — the same technique used by the existing
// WebView2 wallpaper system, but now for our GPU renderer.

use windows::Win32::{
    Foundation::{HWND, LPARAM, WPARAM},
    UI::WindowsAndMessaging::{
        EnumWindows, FindWindowExW, FindWindowW,
        SendMessageTimeoutW, SetParent, SMTO_NORMAL,
    },
};
use windows::core::{BOOL, PCWSTR};

/// Find the WorkerW window behind the desktop icons.
/// This is the window we parent our render surface into for wallpapers.
pub fn find_worker_w() -> Option<HWND> {
    unsafe {
        // Find Progman window.
        let progman = FindWindowW(
            PCWSTR(to_wide("Progman").as_ptr()),
            PCWSTR::null(),
        ).ok()?;

        // Send the magic message that spawns WorkerW.
        let mut _result = 0usize;
        let _ = SendMessageTimeoutW(
            progman,
            0x052C, // Magic message
            WPARAM(0),
            LPARAM(0),
            SMTO_NORMAL,
            1000,
            Some(&mut _result),
        );

        // Find the WorkerW behind the desktop icons.
        let mut worker_w = HWND::default();
        let worker_ptr = &mut worker_w as *mut HWND;

        let _ = EnumWindows(Some(enum_windows_callback), LPARAM(worker_ptr as isize));

        if worker_w.0.is_null() {
            None
        } else {
            Some(worker_w)
        }
    }
}

unsafe extern "system" fn enum_windows_callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let worker_ptr = &mut *(lparam.0 as *mut HWND);

    // Check if this is a WorkerW window.
    let shell_view = FindWindowExW(
        Some(hwnd),
        None,
        PCWSTR(to_wide("SHELLDLL_DefView").as_ptr()),
        PCWSTR::null(),
    );

    if shell_view.is_ok() {
        // The next WorkerW sibling is the one behind the desktop icons.
        if let Ok(next_worker) = FindWindowExW(
            None,
            Some(hwnd),
            PCWSTR(to_wide("WorkerW").as_ptr()),
            PCWSTR::null(),
        ) {
            *worker_ptr = next_worker;
        }
    }

    BOOL(1) // Continue enumeration.
}

/// Embed a window handle into the WorkerW layer.
/// The window will render behind desktop icons.
pub fn embed_in_desktop(child_hwnd: HWND) -> bool {
    if let Some(worker_w) = find_worker_w() {
        unsafe {
            // Set the render window as a child of WorkerW.
            let result = SetParent(child_hwnd, Some(worker_w));
            return result.is_ok();
        }
    }
    false
}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

