// PRISM Installer
//
// Installs PRISM runtime to C:\Program Files\PRISM\
// Includes:
//   - prism.exe (main CLI tool)
//   - PRISM library files
//   - PATH configuration
//   - Uninstaller

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

const INSTALL_DIR: &str = r"C:\Program Files\PRISM";
const UNINSTALL_NAME: &str = "PRISM";

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() > 1 {
        match args[1].as_str() {
            "--uninstall" => {
                if let Err(e) = uninstall() {
                    eprintln!("[ERROR] Uninstall failed: {}", e);
                    std::process::exit(1);
                }
                println!("PRISM has been uninstalled successfully.");
                std::process::exit(0);
            }
            "--help" => {
                print_help();
                return;
            }
            _ => {}
        }
    }
    
    // Install PRISM
    println!("╔════════════════════════════════════════════╗");
    println!("║      PRISM Runtime - Installer v0.1.0      ║");
    println!("╚════════════════════════════════════════════╝\n");
    
    if let Err(e) = install() {
        eprintln!("[ERROR] Installation failed: {}", e);
        std::process::exit(1);
    }
    
    println!("\n✓ PRISM has been installed successfully!");
    println!("  Location: {}", INSTALL_DIR);
    println!("\nYou can now use 'prism' from anywhere:");
    println!("  prism -c widget.html    (compile)");
    println!("  prism -r widget.prd     (run)");
    println!("  prism --help            (show commands)");
}

fn install() -> std::io::Result<()> {
    let current_exe = env::current_exe()?;
    let install_path = Path::new(INSTALL_DIR);
    
    // Step 1: Create installation directory
    println!("\n[1/4] Creating installation directory...");
    if !install_path.exists() {
        fs::create_dir_all(install_path)?;
        println!("  ✓ Created: {}", INSTALL_DIR);
    } else {
        println!("  ℹ Directory already exists: {}", INSTALL_DIR);
    }
    
    // Step 2: Copy executables
    println!("\n[2/4] Installing executables...");
    
    // Get the directory of the current exe (installer location)
    let installer_dir = current_exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Cannot determine installer directory")
    })?;
    
    // Copy prism.exe from same directory as installer
    let prism_src = installer_dir.join("prism.exe");
    if prism_src.exists() {
        let prism_dst = install_path.join("prism.exe");
        fs::copy(&prism_src, &prism_dst)?;
        println!("  ✓ Copied prism.exe");
    } else {
        println!("  ⚠ Warning: prism.exe not found at {}", prism_src.display());
    }
    
    // Step 3: Add to PATH
    println!("\n[3/4] Updating system PATH...");
    if let Err(e) = add_to_path(install_path) {
        println!("  ⚠ Warning: Could not update PATH: {}", e);
        println!("  You may need to manually add {} to your PATH", INSTALL_DIR);
    } else {
        println!("  ✓ Added {} to user PATH", INSTALL_DIR);
    }
    
    // Step 4: Create uninstaller shortcut
    println!("\n[4/4] Setting up uninstaller...");
    create_uninstaller_info(install_path)?;
    println!("  ✓ Uninstaller created at: {}\\uninstall.bat", INSTALL_DIR);
    
    Ok(())
}

fn uninstall() -> std::io::Result<()> {
    let install_path = Path::new(INSTALL_DIR);
    
    println!("Uninstalling PRISM...");
    
    // Remove from PATH
    if let Err(e) = remove_from_path(install_path) {
        eprintln!("Warning: Could not remove from PATH: {}", e);
    }
    
    // Delete installation directory
    if install_path.exists() {
        fs::remove_dir_all(install_path)?;
    }
    
    Ok(())
}

fn add_to_path(install_dir: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;
        use windows::Win32::System::Registry::*;

        let install_str = install_dir.to_string_lossy().to_string();

        unsafe {
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

            if current_path
                .split(';')
                .any(|p| p.eq_ignore_ascii_case(&install_str))
            {
                RegCloseKey(key_handle);
                return Ok(());
            }

            let new_path = if current_path.is_empty() {
                install_str
            } else {
                format!("{};{}", current_path, install_str)
            };

            let new_path_wide: Vec<u16> = OsStr::new(&new_path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let new_path_bytes = std::slice::from_raw_parts(
                new_path_wide.as_ptr() as *const u8,
                new_path_wide.len() * 2,
            );

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
    {
        Ok(())
    }
}

fn remove_from_path(install_dir: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;
        use windows::Win32::System::Registry::*;

        let install_str = install_dir.to_string_lossy().to_string();

        unsafe {
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

            let new_path = current_path
                .split(';')
                .filter(|p| !p.eq_ignore_ascii_case(&install_str))
                .collect::<Vec<_>>()
                .join(";");

            let new_path_wide: Vec<u16> = OsStr::new(&new_path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect();

            let new_path_bytes = std::slice::from_raw_parts(
                new_path_wide.as_ptr() as *const u8,
                new_path_wide.len() * 2,
            );

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
    {
        Ok(())
    }
}

fn create_uninstaller_info(install_path: &Path) -> std::io::Result<()> {
    // Create a simple batch uninstaller script
    let uninstall_bat = install_path.join("uninstall.bat");
    let installer_path = install_path.join("prism-installer.exe");
    
    let content = format!(
        "@echo off\necho Uninstalling PRISM...\n\"{:}\" --uninstall\npause\n",
        installer_path.display()
    );
    
    fs::write(uninstall_bat, content)?;
    Ok(())
}

fn print_help() {
    println!("PRISM Installer - v0.1.0");
    println!("\nUsage: prism-installer [OPTION]");
    println!("\nOptions:");
    println!("  (no args)    Install PRISM to C:\\Program Files\\PRISM\\");
    println!("  --uninstall  Remove PRISM from your system");
    println!("  --help       Show this help message");
}
