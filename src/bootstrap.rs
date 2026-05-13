// PRISM CLI Bootstrap
//
// Handles one-time installation:
//   1. Copy prism.exe to C:\Program Files\PRISM\
//   2. Add to user PATH (HKCU\Environment\Path)
//   3. Relaunch from installed location

use std::env;
use std::fs;
use std::path::Path;
use std::process::Command;

const INSTALL_DIR: &str = r"C:\Program Files\PRISM";

/// Check if bootstrap is needed (running from non-standard location)
pub fn should_bootstrap() -> bool {
    let current_exe = match env::current_exe() {
        Ok(exe) => exe,
        Err(_) => return false,
    };

    let current_dir = match current_exe.parent() {
        Some(dir) => dir.to_string_lossy().to_lowercase(),
        None => return false,
    };

    let install_dir_lower = INSTALL_DIR.to_lowercase();
    !current_dir.contains(&install_dir_lower)
}

/// Perform bootstrap: install to Program Files and relaunch
pub fn bootstrap() -> std::io::Result<()> {
    let current_exe = env::current_exe()?;
    let install_path = Path::new(INSTALL_DIR);
    let target_exe = install_path.join("prism.exe");

    // Create directory if it doesn't exist
    if !install_path.exists() {
        fs::create_dir_all(install_path)?;
    }

    // Copy exe to Program Files
    println!("[PRISM] Installing to {}...", INSTALL_DIR);
    fs::copy(&current_exe, &target_exe)?;
    println!("[PRISM] ✓ Installed to {}", target_exe.display());

    // Try to add to PATH (non-blocking if it fails)
    if let Ok(()) = add_to_path(install_path) {
        println!("[PRISM] ✓ Added to system PATH");
    } else {
        println!("[PRISM] ℹ To add PRISM to PATH, run: prism --setup-env");
    }

    // Relaunch from installed location with same arguments
    let args: Vec<String> = env::args().skip(1).collect();
    println!("[PRISM] Relaunching from installed location...\n");

    Command::new(&target_exe)
        .args(&args)
        .spawn()?;

    // Exit current process
    std::process::exit(0);
}

/// Add PRISM to user PATH environment variable (no elevation needed)
#[cfg(target_os = "windows")]
fn add_to_path(install_dir: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;
    use windows::Win32::System::Registry::*;

    let install_str = install_dir.to_string_lossy().to_string();

    unsafe {
        // Open HKCU\Environment (user PATH, no elevation needed)
        let mut key_handle = std::mem::zeroed();
        let subkey: Vec<u16> = OsStr::new("Environment")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(subkey.as_ptr()),
            None,
            KEY_READ | KEY_WRITE,
            &mut key_handle,
        );

        if result.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::PermissionDenied,
                "Could not open HKCU\\Environment",
            ));
        }

        // Read current PATH value
        let mut path_buffer = vec![0u16; 16384];
        let mut size = path_buffer.len() as u32 * 2;
        let value_name: Vec<u16> = OsStr::new("Path")
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let result = RegQueryValueExW(
            key_handle,
            windows::core::PCWSTR(value_name.as_ptr()),
            None,
            None,
            Some(path_buffer.as_mut_ptr() as *mut u8),
            Some(&mut size),
        );

        let current_path = if result.is_ok() {
            let len = (size as usize / 2).saturating_sub(1);
            String::from_utf16_lossy(&path_buffer[..len]).to_string()
        } else {
            String::new()
        };

        // Check if already in PATH
        if current_path
            .split(';')
            .any(|p| p.eq_ignore_ascii_case(&install_str))
        {
            RegCloseKey(key_handle);
            return Ok(());
        }

        // Append to PATH
        let new_path = if current_path.is_empty() {
            install_str
        } else {
            format!("{};{}", current_path, install_str)
        };

        let new_path_wide: Vec<u16> = OsStr::new(&new_path)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();

        let new_path_bytes = {
            std::slice::from_raw_parts(
                new_path_wide.as_ptr() as *const u8,
                new_path_wide.len() * 2,
            )
        };

        let result = RegSetValueExW(
            key_handle,
            windows::core::PCWSTR(value_name.as_ptr()),
            None,
            REG_SZ,
            Some(new_path_bytes),
        );

        RegCloseKey(key_handle);

        if result.is_err() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                "Failed to update PATH in registry",
            ));
        }
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn add_to_path(_install_dir: &Path) -> std::io::Result<()> {
    // On non-Windows, just silently succeed
    Ok(())
}
