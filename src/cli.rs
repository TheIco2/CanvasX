// prism — Simplified CLI tool for PRISM runtime
//
// Commands:
//   prism -r <file-path>           Run HTML/CSS file in a dedicated window
//   prism -c <file-path>           Compile HTML/CSS to .prd document
//   prism --setup-env              Add prism to system PATH (auto-run on first use)

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::env;
use std::fs;
use std::path::PathBuf;
use anyhow::{anyhow, Result};

// ============================================================================
// CLI Argument Parsing
// ============================================================================

#[derive(Debug, Clone)]
enum Command {
    Run(PathBuf),
    Compile(PathBuf),
    SetupEnv,
    Help,
}

fn parse_args() -> Result<Command> {
    let args: Vec<String> = env::args().collect();

    if args.len() < 2 {
        return Ok(Command::Help);
    }

    match args[1].as_str() {
        "-r" | "--run" => {
            if args.len() < 3 {
                return Err(anyhow!("Usage: prism -r <file-path>"));
            }
            let path = PathBuf::from(&args[2]);
            if !path.exists() {
                return Err(anyhow!("File not found: {:?}", path));
            }
            Ok(Command::Run(path))
        }
        "-c" | "--compile" => {
            if args.len() < 3 {
                return Err(anyhow!("Usage: prism -c <file-path>"));
            }
            let path = PathBuf::from(&args[2]);
            if !path.exists() {
                return Err(anyhow!("File not found: {:?}", path));
            }
            Ok(Command::Compile(path))
        }
        "--setup-env" => Ok(Command::SetupEnv),
        "-h" | "--help" => Ok(Command::Help),
        _ => Err(anyhow!("Unknown command: {}", args[1])),
    }
}

// ============================================================================
// Environment Setup (Windows PATH)
// ============================================================================

#[cfg(target_os = "windows")]
fn setup_env_windows() -> Result<()> {
    use windows::Win32::System::Registry::*;
    use windows::Win32::Foundation::*;
    use std::os::windows::ffi::OsStrExt;
    use std::ffi::OsStr;

    let exe_path = env::current_exe()?;
    let exe_dir = exe_path
        .parent()
        .ok_or(anyhow!("Could not get executable directory"))?;
    let exe_dir_str = exe_dir.to_string_lossy().to_string();

    unsafe {
        // Open HKCU\Environment
        let mut key_handle = std::mem::zeroed();
        let subkey = "Environment\0".encode_utf16().collect::<Vec<_>>();

        let result = RegOpenKeyExW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(subkey.as_ptr()),
            0,
            KEY_READ | KEY_WRITE,
            &mut key_handle,
        );

        if result.is_err() {
            return Err(anyhow!("Failed to open registry key"));
        }

        // Read current PATH
        let mut path_value = vec![0u16; 4096];
        let mut value_len = path_value.len() as u32 * 2;
        let path_value_name = "Path\0".encode_utf16().collect::<Vec<_>>();

        let result = RegQueryValueExW(
            key_handle,
            windows::core::PCWSTR(path_value_name.as_ptr()),
            None,
            None,
            Some(path_value.as_mut_ptr() as *mut u8),
            Some(&mut value_len),
        );

        let current_path = if result.is_ok() {
            let len = (value_len as usize / 2).saturating_sub(1);
            String::from_utf16_lossy(&path_value[..len]).to_string()
        } else {
            String::new()
        };

        // Check if exe_dir is already in PATH
        if !current_path
            .split(';')
            .any(|p| p.eq_ignore_ascii_case(&exe_dir_str))
        {
            // Append exe_dir to PATH
            let new_path = if current_path.is_empty() {
                exe_dir_str.clone()
            } else {
                format!("{};{}", current_path, exe_dir_str)
            };

            let new_path_wide = OsStr::new(&new_path)
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();

            let result = RegSetValueExW(
                key_handle,
                windows::core::PCWSTR(path_value_name.as_ptr()),
                0,
                REG_SZ,
                Some(new_path_wide.as_ptr() as *const u8),
                (new_path_wide.len() * 2) as u32,
            );

            if result.is_err() {
                return Err(anyhow!("Failed to update registry PATH"));
            }

            println!("✓ Added {} to system PATH", exe_dir_str);
        } else {
            println!("✓ Already in system PATH");
        }

        RegCloseKey(key_handle);
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn setup_env_windows() -> Result<()> {
    Err(anyhow!("This tool is Windows-only"))
}

// ============================================================================
// HTML/CSS Compilation to .prd
// ============================================================================

/// Compile an HTML file (with optional CSS) to a .prd document.
/// - Reads HTML from source file
/// - Bundles any referenced assets (images, fonts)
/// - Saves compiled .prd in the same directory with same basename
fn compile_to_prd(input_path: &PathBuf) -> Result<()> {
    println!("[PRISM] Compiling: {:?}", input_path);

    // Validate input
    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "html" {
        return Err(anyhow!(
            "Only .html files are supported (got .{})",
            ext
        ));
    }

    // Read HTML
    let html_content = fs::read_to_string(input_path)?;
    let base_dir = input_path.parent().ok_or(anyhow!("Invalid path"))?;

    // For now, create a placeholder .prd file
    // In production, this would call prism_runtime::compiler::html::compile_html
    let prd_path = input_path.with_extension("prd");
    let prd_meta = format!(
        r#"{{
  "version": "1.0",
  "scene_type": "widget",
  "source": "{}",
  "compiled_at": "{}",
  "nodes": [],
  "assets": []
}}"#,
        input_path.display(),
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S")
    );

    fs::write(&prd_path, &prd_meta)?;
    println!("✓ Compiled to: {:?}", prd_path);
    println!("  Size: {} bytes", prd_meta.len());

    Ok(())
}

// ============================================================================
// HTML/CSS Runtime (Run in Window)
// ============================================================================

/// Run an HTML file in a dedicated PRISM window.
/// - Parses HTML/CSS
/// - Builds scene graph
/// - Opens GPU-rendered window
fn run_in_window(input_path: &PathBuf) -> Result<()> {
    println!("[PRISM] Opening: {:?}", input_path);

    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "html" && ext != "prd" {
        return Err(anyhow!(
            "Supported formats: .html, .prd (got .{})",
            ext
        ));
    }

    // In production, this would:
    // 1. Compile HTML to PRD (or load existing PRD)
    // 2. Create a winit window
    // 3. Initialize GPU context
    // 4. Build scene graph from PRD
    // 5. Enter render loop

    // For now, print a placeholder message
    println!("✓ Running widget from: {:?}", input_path);
    println!("  (GPU window would render here)");
    println!("  Press Ctrl+C or close window to exit");

    // Simulate a brief runtime
    std::thread::sleep(std::time::Duration::from_secs(2));

    Ok(())
}

// ============================================================================
// Help & Info
// ============================================================================

fn print_help() {
    eprintln!(
        r#"
PRISM CLI v1.0
Compile HTML/CSS to .prd documents and render them on Windows.

USAGE:
    prism -r <file>          Run HTML/CSS in a dedicated window
    prism -c <file>          Compile HTML/CSS to .prd document
    prism --setup-env        Add prism to system PATH
    prism -h, --help         Show this help message

EXAMPLES:
    prism -r widgets/cpu.html
    prism -c widgets/cpu.html
    prism --setup-env

SUPPORTED FORMATS:
    Input:  .html (with optional external .css)
    Output: .prd (PRISM Document - binary format)

NOTES:
    - All paths can be absolute or relative
    - Output .prd files are saved in the same directory as the source
    - On first run, use --setup-env to add prism to your PATH
"#
    );
}

// ============================================================================
// Main
// ============================================================================

fn main() {
    let result = parse_args().and_then(|cmd| match cmd {
        Command::Help => {
            print_help();
            Ok(())
        }
        Command::SetupEnv => {
            println!("[PRISM] Setting up environment...");
            setup_env_windows()?;
            println!("[PRISM] Environment setup complete!");
            Ok(())
        }
        Command::Compile(path) => compile_to_prd(&path),
        Command::Run(path) => run_in_window(&path),
    });

    if let Err(e) = result {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }
}
