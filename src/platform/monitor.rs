// prism-runtime/src/platform/monitor.rs
//
// Multi-monitor enumeration and DPI-aware monitor info.

use windows::Win32::{
    Foundation::{LPARAM, RECT},
    Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFOEXW,
    },
    UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI},
};
use windows::core::BOOL;

/// Information about a connected display monitor.
#[derive(Debug, Clone)]
pub struct MonitorInfo {
    /// Monitor handle (for platform operations).
    pub handle: isize,
    /// Monitor name.
    pub name: String,
    /// Position and size in virtual screen coordinates.
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// DPI scale factor (e.g. 1.0, 1.25, 1.5, 2.0).
    pub scale_factor: f64,
    /// Is this the primary monitor?
    pub primary: bool,
}

/// Enumerate all connected monitors.
pub fn enumerate_monitors() -> Vec<MonitorInfo> {
    let mut monitors: Vec<MonitorInfo> = Vec::new();
    let monitors_ptr = &mut monitors as *mut Vec<MonitorInfo>;

    unsafe {
        let _ = EnumDisplayMonitors(
            None,
            None,
            Some(enum_monitor_callback),
            LPARAM(monitors_ptr as isize),
        );
    }

    monitors
}

unsafe extern "system" fn enum_monitor_callback(
    hmonitor: HMONITOR,
    _hdc: HDC,
    _rect: *mut RECT,
    lparam: LPARAM,
) -> BOOL {
    let monitors = &mut *(lparam.0 as *mut Vec<MonitorInfo>);

    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;

    if GetMonitorInfoW(hmonitor, &mut info as *mut _ as *mut _).as_bool() {
        let rc = &info.monitorInfo.rcMonitor;

        let name = String::from_utf16_lossy(
            &info.szDevice[..info.szDevice.iter().position(|&c| c == 0).unwrap_or(info.szDevice.len())]
        );

        let primary = (info.monitorInfo.dwFlags & 1) != 0; // MONITORINFOF_PRIMARY

        // Get DPI
        let mut dpi_x = 96u32;
        let mut dpi_y = 96u32;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale_factor = dpi_x as f64 / 96.0;

        monitors.push(MonitorInfo {
            handle: hmonitor.0 as isize,
            name,
            x: rc.left,
            y: rc.top,
            width: (rc.right - rc.left) as u32,
            height: (rc.bottom - rc.top) as u32,
            scale_factor,
            primary,
        });
    }

    BOOL(1)
}

