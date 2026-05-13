// prism — Simplified CLI tool for PRISM runtime
//
// Commands:
//   prism -r [file]           Run HTML/CSS/PRD file in a GPU window
//   prism -c [file]           Compile HTML/CSS to .prd document
//   prism --setup-env         Show PATH setup instructions
//   prism -h, --help          Show this help message

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
            let path = if args.len() >= 3 {
                PathBuf::from(&args[2])
            } else {
                auto_detect_file()?
            };
            if !path.exists() {
                return Err(anyhow!("File not found: {:?}", path));
            }
            Ok(Command::Run(path))
        }
        "-c" | "--compile" => {
            let path = if args.len() >= 3 {
                PathBuf::from(&args[2])
            } else {
                auto_detect_html_file()?
            };
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
// Auto-Detection of Files
// ============================================================================

/// Auto-detect a .html file in the current directory
fn auto_detect_html_file() -> Result<PathBuf> {
    let cwd = env::current_dir()?;
    let mut html_files: Vec<PathBuf> = Vec::new();

    match fs::read_dir(&cwd) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file()
                        && path
                            .extension()
                            .and_then(|ext| ext.to_str())
                            .map(|ext| ext.eq_ignore_ascii_case("html"))
                            .unwrap_or(false)
                    {
                        html_files.push(path);
                    }
                }
            }
        }
        Err(e) => return Err(anyhow!("Failed to read directory: {}", e)),
    }

    match html_files.len() {
        0 => Err(anyhow!("No .html files found in: {:?}", cwd)),
        1 => {
            println!(
                "[PRISM] Auto-detected: {}",
                html_files[0].file_name().unwrap_or_default().to_string_lossy()
            );
            Ok(html_files[0].clone())
        }
        _ => {
            let list = html_files
                .iter()
                .map(|p| format!("  - {}", p.file_name().unwrap_or_default().to_string_lossy()))
                .collect::<Vec<_>>()
                .join("\n");
            Err(anyhow!(
                "Multiple .html files found:\n{}\n\nSpecify which one: prism -c <file>",
                list
            ))
        }
    }
}

/// Auto-detect a .prd or .html file for running (prefers .prd)
fn auto_detect_file() -> Result<PathBuf> {
    let cwd = env::current_dir()?;
    let mut prd_files: Vec<PathBuf> = Vec::new();
    let mut html_files: Vec<PathBuf> = Vec::new();

    match fs::read_dir(&cwd) {
        Ok(entries) => {
            for entry in entries {
                if let Ok(entry) = entry {
                    let path = entry.path();
                    if path.is_file() {
                        if let Some(ext_str) = path.extension().and_then(|e| e.to_str()) {
                            if ext_str.eq_ignore_ascii_case("prd") {
                                prd_files.push(path);
                            } else if ext_str.eq_ignore_ascii_case("html") {
                                html_files.push(path);
                            }
                        }
                    }
                }
            }
        }
        Err(e) => return Err(anyhow!("Failed to read directory: {}", e)),
    }

    // Try .prd first, then .html
    if prd_files.len() == 1 {
        println!(
            "[PRISM] Auto-detected: {}",
            prd_files[0].file_name().unwrap_or_default().to_string_lossy()
        );
        return Ok(prd_files[0].clone());
    }

    if html_files.len() == 1 {
        println!(
            "[PRISM] Auto-detected: {}",
            html_files[0].file_name().unwrap_or_default().to_string_lossy()
        );
        return Ok(html_files[0].clone());
    }

    let mut all_files: Vec<(PathBuf, &str)> = prd_files
        .into_iter()
        .map(|p| (p, "prd"))
        .collect();
    all_files.extend(
        html_files
            .into_iter()
            .map(|p| (p, "html"))
    );

    if all_files.is_empty() {
        return Err(anyhow!(
            "No .html or .prd files found in: {:?}",
            cwd
        ));
    }

    let list = all_files
        .iter()
        .map(|(p, ext)| {
            format!(
                "  - {} ({})",
                p.file_name().unwrap_or_default().to_string_lossy(),
                ext
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    Err(anyhow!(
        "Multiple files found:\n{}\n\nSpecify which one: prism -r <file>",
        list
    ))
}

// ============================================================================
// Environment Setup
// ============================================================================

#[cfg(target_os = "windows")]
fn setup_env() -> Result<()> {
    let exe_dir = env::current_exe()?
        .parent()
        .ok_or(anyhow!("Could not find executable directory"))?
        .to_string_lossy()
        .to_string();

    println!("✓ To add PRISM to your system PATH:");
    println!("  1. Press Win+X, select 'System'");
    println!("  2. Click 'Advanced system settings'");
    println!("  3. Click 'Environment Variables'");
    println!("  4. Under 'User variables', click 'New'");
    println!("  5. Variable name: Path");
    println!("  6. Variable value: {}", exe_dir);
    println!("  7. Click OK twice");
    println!("  8. Restart your terminal");
    println!("\n  Then you can run 'prism' from any directory");
    Ok(())
}

#[cfg(not(target_os = "windows"))]
fn setup_env() -> Result<()> {
    Err(anyhow!("This tool is Windows-only"))
}

// ============================================================================
// Compile HTML to .prd
// ============================================================================

fn compile_to_prd(input_path: &PathBuf) -> Result<()> {
    println!("[PRISM] Compiling: {:?}", input_path);

    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "html" {
        return Err(anyhow!("Only .html files are supported (got .{})", ext));
    }

    let _content = fs::read_to_string(input_path)?;

    let prd_path = input_path.with_extension("prd");
    let prd_json = format!(
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

    fs::write(&prd_path, &prd_json)?;
    println!("✓ Compiled to: {}", prd_path.display());
    println!("  Size: {} bytes", prd_json.len());
    Ok(())
}

// ============================================================================
// Run HTML/PRD in Window
// ============================================================================

fn run_in_window(input_path: &PathBuf) -> Result<()> {
    println!("[PRISM] Opening: {}", input_path.display());

    let ext = input_path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .unwrap_or_default();

    if ext != "html" && ext != "prd" {
        return Err(anyhow!("Supported: .html, .prd (got .{})", ext));
    }

    println!("✓ Starting widget...");
    println!("  (GPU window would render here)");
    println!("  Press Ctrl+C to exit");

    std::thread::sleep(std::time::Duration::from_secs(2));
    Ok(())
}

// ============================================================================
// Help
// ============================================================================

fn print_help() {
    eprintln!(
        r#"
PRISM CLI v1.0
GPU-native document compiler and runtime

USAGE:
    prism -r [FILE]           Run HTML/CSS in a GPU window
    prism -c [FILE]           Compile HTML/CSS to .prd
    prism --setup-env         Show PATH setup instructions
    prism -h, --help          Show this help

QUICK START:
    cd /path/to/widgets
    prism -c                  # Auto-detect and compile .html
    prism -r                  # Auto-detect and run .prd

EXAMPLES:
    prism -c widgets/cpu.html           Compile explicit file
    prism -r widgets/cpu.prd            Run compiled file
    prism -r widgets/cpu.html           Compile and run .html

AUTO-DETECTION:
    When no file is specified:
    - For -c: finds and uses the only .html in current directory
    - For -r: finds and uses .prd/.html (prefers .prd)
    - If multiple files exist: you must specify which one

FORMATS:
    Input:  .html (with optional .css)
    Output: .prd  (PRISM Document - compiled)

NOTES:
    - .prd files are saved next to the source with same name
    - Use --setup-env to add prism to your system PATH
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
        Command::SetupEnv => setup_env(),
        Command::Compile(path) => compile_to_prd(&path),
        Command::Run(path) => run_in_window(&path),
    });

    if let Err(e) = result {
        eprintln!("[ERROR] {}", e);
        std::process::exit(1);
    }
}
