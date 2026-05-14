// prism — Simplified CLI tool for PRISM runtime
//
// Commands:
//   prism -r [file]           Run HTML/CSS/PRD file in a GPU window
//   prism -c [file]           Compile HTML/CSS to .prd document
//   prism --setup-env         Show PATH setup instructions
//   prism -h, --help          Show this help message

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

    // Use the library's real compiler to generate actual scene graph
    prism_runtime::compiler::compile_html_file(input_path)?;

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

    if ext == "html" {
        // If .html is provided, compile to .prd first
        println!("[PRISM] Compiling HTML to .prd...");
        compile_to_prd(input_path)?;
        
        // Now open the generated .prd
        let prd_path = input_path.with_extension("prd");
        return render_prd(&prd_path);
    }

    if ext == "prd" {
        return render_prd(input_path);
    }

    Err(anyhow!("Supported: .html, .prd (got .{})", ext))
}

/// Extract viewport dimensions from HTML meta tag
fn extract_viewport_size(html_path: &PathBuf) -> Option<(u32, u32)> {
    if !html_path.exists() {
        return None;
    }
    
    let html = fs::read_to_string(html_path).ok()?;
    
    // Look for: <meta name="prism:viewport" content="WIDTHxHEIGHT">
    let re = regex::Regex::new(r#"<meta\s+name="prism:viewport"\s+content="(\d+)x(\d+)""#).ok()?;
    if let Some(caps) = re.captures(&html) {
        if let (Ok(w), Ok(h)) = (
            caps.get(1)?.as_str().parse::<u32>(),
            caps.get(2)?.as_str().parse::<u32>(),
        ) {
            return Some((w, h));
        }
    }
    
    None
}

/// Render a .prd document in a GPU window
fn render_prd(prd_path: &PathBuf) -> Result<()> {
    println!("[PRISM] Loading PRD: {}", prd_path.display());

    let prd_data = fs::read(prd_path)?;
    let doc = prism_runtime::PrdDocument::from_binary(&prd_data)
        .map_err(|e| anyhow!("Failed to load .prd: {}", e))?;
    
    // Try to get custom viewport size from HTML meta tags
    let (viewport_w, viewport_h) = if let Some(parent) = prd_path.parent() {
        let stem = prd_path.file_stem().unwrap_or_default();
        let html_path = parent.join(format!("{}.html", stem.to_string_lossy()));
        
        if let Some((w, h)) = extract_viewport_size(&html_path) {
            (w, h)
        } else {
            (doc.viewport_width as u32, doc.viewport_height as u32)
        }
    } else {
        (doc.viewport_width as u32, doc.viewport_height as u32)
    };

    println!("\n📦 PRD Document Loaded:");
    println!("  Name: {}", doc.meta.name);
    println!("  Type: {:?}", doc.meta.scene_type);
    println!("  Nodes: {}", doc.nodes.len());
    println!("  Assets: {} items", doc.assets.images.len() + doc.assets.fonts.len());
    println!("  Viewport: {}x{}", viewport_w, viewport_h);
    println!("\n→ Opening GPU window...\n");

    // Render using GPU
    render_document_in_window(&doc, viewport_w, viewport_h)?;
    
    println!("\n✓ Widget closed");
    Ok(())
}

/// Render a PRD document in a GPU window using wgpu + winit
fn render_document_in_window(doc: &prism_runtime::PrdDocument, viewport_w: u32, viewport_h: u32) -> Result<()> {
    use winit::event_loop::EventLoop;
    use winit::window::WindowAttributes;
    use winit::dpi::PhysicalSize;
    use winit::keyboard::{Key, NamedKey};
    use std::sync::Arc;

    // Create event loop
    let event_loop = EventLoop::new()?;
    
    // Create window with custom viewport size
    let window = Arc::new(
        event_loop.create_window(
            WindowAttributes::default()
                .with_title(&doc.meta.name)
                .with_inner_size(PhysicalSize::new(viewport_w, viewport_h))
        )?
    );

    // Create GPU context
    let gpu_ctx = pollster::block_on(prism_runtime::GpuContext::new(window.clone()))?;
    let mut renderer = prism_runtime::gpu::renderer::Renderer::new(&gpu_ctx)?;
    
    // Create scene graph from document
    let mut scene = prism_runtime::SceneGraph::new(doc.clone());

    println!("✓ GPU context initialized (backend: {:?})", gpu_ctx.backend);
    println!("✓ Scene graph created with {} nodes", doc.nodes.len());
    println!();
    println!("Press ESC or close the window to exit...");
    println!();

    let mut running = true;
    let mut last_frame_time = std::time::Instant::now();

    event_loop.run(move |event, target| {
        use winit::event::{Event, WindowEvent};

        match event {
            Event::WindowEvent { window_id, event } if window_id == window.id() => {
                match event {
                    WindowEvent::CloseRequested => {
                        running = false;
                        target.exit();
                    }
                    WindowEvent::KeyboardInput { event, .. } => {
                        if event.logical_key == Key::Named(NamedKey::Escape) {
                            running = false;
                            target.exit();
                        }
                    }
                    WindowEvent::RedrawRequested => {
                        if !running {
                            return;
                        }

                        // Calculate delta time
                        let now = std::time::Instant::now();
                        let dt = (now - last_frame_time).as_secs_f32();
                        last_frame_time = now;

                        // Tick scene (layout, animate, paint) using desired viewport size
                        scene.tick(
                            viewport_w as f32,
                            viewport_h as f32,
                            dt,
                            &mut renderer.font_system,
                            window.scale_factor() as f32,
                        );

                        // Get instances and text areas
                        let instances = &scene.cached_instances;
                        let text_areas = scene.text_areas();
                        let clear_color = prism_runtime::prd::value::Color::BLACK;

                        // Render
                        match renderer.render(&gpu_ctx, instances, text_areas, clear_color) {
                            Ok(_) => {},
                            Err(wgpu::SurfaceError::Outdated) => {
                                // Window was resized, reconfigure surface
                                let size = window.inner_size();
                                gpu_ctx.surface.configure(&gpu_ctx.device, &wgpu::SurfaceConfiguration {
                                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                                    format: gpu_ctx.surface_format,
                                    width: size.width.max(1),
                                    height: size.height.max(1),
                                    present_mode: gpu_ctx.surface_config.present_mode,
                                    alpha_mode: gpu_ctx.surface_config.alpha_mode,
                                    desired_maximum_frame_latency: gpu_ctx.surface_config.desired_maximum_frame_latency,
                                    view_formats: vec![],
                                });
                            }
                            Err(e) => {
                                eprintln!("GPU render error: {:?}", e);
                                running = false;
                                target.exit();
                            }
                        }
                    }
                    _ => {}
                }
            }
            Event::AboutToWait => {
                window.request_redraw();
            }
            _ => {}
        }
    })?;

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
