// Build script for PRISM Runtime
// Handles resource embedding for Windows executables

fn main() {
    // Only embed icon on Windows
    #[cfg(target_os = "windows")]
    {
        let icon_path = "assets/prism-icon.ico";
        
        // Check if icon file exists
        if std::path::Path::new(icon_path).exists() {
            // Use winresource to embed the icon
            let mut res = winresource::WindowsResource::new();
            res.set_icon(icon_path);
            
            // Set version info
            res.set("ProductName", "PRISM Runtime");
            res.set("FileDescription", "PRISM - GPU-native 2D scene graph renderer");
            res.set("ProductVersion", "0.1.0");
            res.set("FileVersion", "0.1.0");
            
            if let Err(e) = res.compile() {
                eprintln!("Warning: Failed to compile Windows resources: {}", e);
                eprintln!("The executable will build without an icon.");
            }
        } else {
            eprintln!("Warning: Icon file not found at {}", icon_path);
            eprintln!("Run: python convert-icon.py");
            eprintln!("The executable will build without an icon.");
        }
    }
    
    println!("cargo:rerun-if-changed=assets/prism-icon.ico");
}
