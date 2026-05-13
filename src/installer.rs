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
use std::path::Path;

const INSTALL_DIR: &str = r"C:\Program Files\PRISM";
const BIN_DIR: &str = r"C:\Program Files\PRISM\bin";
const LIB_DIR: &str = r"C:\Program Files\PRISM\lib";

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
    let bin_path = Path::new(BIN_DIR);
    let lib_path = Path::new(LIB_DIR);
    
    // Step 1: Create installation directories
    println!("\n[1/5] Creating installation directories...");
    if !install_path.exists() {
        fs::create_dir_all(install_path)?;
        println!("  ✓ Created: {}", INSTALL_DIR);
    }
    if !bin_path.exists() {
        fs::create_dir_all(bin_path)?;
        println!("  ✓ Created: {}", BIN_DIR);
    }
    if !lib_path.exists() {
        fs::create_dir_all(lib_path)?;
        println!("  ✓ Created: {}", LIB_DIR);
    }
    
    // Step 2: Copy executables
    println!("\n[2/5] Installing executables...");
    
    // Get the directory of the current exe (installer location)
    let installer_dir = current_exe.parent().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "Cannot determine installer directory")
    })?;
    
    // Copy prism.exe to bin/
    let prism_src = installer_dir.join("prism.exe");
    if prism_src.exists() {
        let prism_dst = bin_path.join("prism.exe");
        fs::copy(&prism_src, &prism_dst)?;
        println!("  ✓ Copied prism.exe to bin\\");
    } else {
        println!("  ⚠ Warning: prism.exe not found at {}", prism_src.display());
    }
    
    // Step 3: Copy library files
    println!("\n[3/5] Installing library files...");
    copy_library_files(installer_dir, lib_path)?;
    
    // Step 4: Add to PATH
    println!("\n[4/5] Updating system PATH...");
    if let Err(e) = add_to_path(bin_path) {
        println!("  ⚠ Warning: Could not update PATH: {}", e);
        println!("  You may need to manually add {} to your PATH", BIN_DIR);
    } else {
        println!("  ✓ Added {} to user PATH", BIN_DIR);
    }
    
    // Step 5: Create uninstaller shortcut
    println!("\n[5/5] Setting up uninstaller...");
    create_uninstaller_info(install_path)?;
    println!("  ✓ Uninstaller created at: {}\\uninstall.bat", INSTALL_DIR);
    
    Ok(())
}

fn uninstall() -> std::io::Result<()> {
    let install_path = Path::new(INSTALL_DIR);
    let bin_path = Path::new(BIN_DIR);
    let lib_path = Path::new(LIB_DIR);
    
    println!("Uninstalling PRISM...");
    
    // Remove from PATH
    if let Err(e) = remove_from_path(bin_path) {
        eprintln!("Warning: Could not remove from PATH: {}", e);
    }
    
    // Delete installation directories
    if bin_path.exists() {
        fs::remove_dir_all(bin_path)?;
    }
    if lib_path.exists() {
        fs::remove_dir_all(lib_path)?;
    }
    if install_path.exists() {
        fs::remove_dir_all(install_path)?;
    }
    
    Ok(())
}

fn copy_library_files(source_dir: &Path, lib_path: &Path) -> std::io::Result<()> {
    // Look for library files in the source directory:
    // - prism_runtime.dll
    // - prism_runtime.lib
    // - prism_runtime.rlib
    // - Any other .dll or .lib files
    
    let mut found_any = false;
    
    for entry in fs::read_dir(source_dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();
        
        // Copy DLL and LIB files
        if file_name_str.ends_with(".dll") || 
           file_name_str.ends_with(".lib") || 
           file_name_str.ends_with(".rlib") ||
           file_name_str.contains("prism_runtime") {
            
            if path.is_file() {
                let dest = lib_path.join(&file_name);
                fs::copy(&path, &dest)?;
                println!("  ✓ Copied {}", file_name_str);
                found_any = true;
            }
        }
    }
    
    if !found_any {
        println!("  ℹ No library files found (this is OK for CLI-only installations)");
    }
    
    Ok(())
}

fn add_to_path(bin_path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;
        use windows::Win32::System::Registry::*;

        let install_str = bin_path.to_string_lossy().to_string();

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

fn remove_from_path(bin_path: &Path) -> std::io::Result<()> {
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use std::ffi::OsStr;
        use windows::Win32::System::Registry::*;

        let install_str = bin_path.to_string_lossy().to_string();

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
    // Create a standalone uninstaller script that requests elevation
    let uninstall_bat = install_path.join("uninstall.bat");
    
    // Generate a simpler batch script without complex line continuations
    let content = r#"@echo off
setlocal enabledelayedexpansion

REM Request admin elevation if not already running as admin
net session >nul 2>&1
if %errorLevel% neq 0 (
    echo Requesting administrator privileges...
    powershell -NoProfile -Command "Start-Process cmd.exe -ArgumentList '/c \"%~f0\"' -Verb RunAs" >nul 2>&1
    exit /b
)

echo.
echo Uninstalling PRISM...
echo.

REM Remove from PATH using PowerShell
powershell -NoProfile -Command "Invoke-Expression @' 
`$path = [Environment]::GetEnvironmentVariable('Path', 'User')
`$newPath = (`$path -split ';' | Where-Object { `$_ -and `$_ -ne 'C:\Program Files\PRISM\bin' }) -join ';'
[Environment]::SetEnvironmentVariable('Path', `$newPath, 'User')
Write-Host 'Removed from PATH'
'@" 2>nul

REM Wait a moment for registry to update
timeout /t 1 /nobreak >nul

REM Remove installation directories
echo Deleting installation directories...
if exist "C:\Program Files\PRISM\bin" (
    rmdir /s /q "C:\Program Files\PRISM\bin" 2>nul
    echo - Deleted bin directory
)
if exist "C:\Program Files\PRISM\lib" (
    rmdir /s /q "C:\Program Files\PRISM\lib" 2>nul
    echo - Deleted lib directory
)

REM Remove root directory
if exist "C:\Program Files\PRISM" (
    rmdir /s /q "C:\Program Files\PRISM" 2>nul
    echo - Deleted PRISM directory
)

echo.
echo PRISM has been uninstalled successfully.
echo Please open a new terminal for the PATH changes to take effect.
echo.
pause
"#;
    
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
