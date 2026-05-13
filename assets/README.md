# PRISM Icon Generation

This directory contains the PRISM runtime icon in multiple formats.

## Files

- **`prism-icon.svg`** - Source vector icon (2.8 KB)
  - High-quality scalable design with gradients and effects
  - Can be edited in any vector graphics editor
  
- **`prism-icon.ico`** - Windows icon file (746 bytes)
  - Multi-resolution icon for Windows executables
  - Contains sizes: 16×16, 32×32, 48×48, 64×64, 128×128, 256×256
  - Currently generated using high-quality PIL rendering

## Regenerating the Icon

To regenerate `prism-icon.ico` from the SVG:

```bash
# Basic usage (uses best available method)
python convert-icon.py

# High-quality PIL rendering (recommended for quick regeneration)
python convert-icon-hq.py
```

## Quality Levels (Best to Worst)

### 1. **ImageMagick** (Professional Quality) ⭐⭐⭐
Recommended for production releases.

**Installation:**
- Windows: Download from https://imagemagick.org/script/download.php
- Select "Visual C++" installer
- After installation, restart your terminal

**Command:**
```bash
python convert-icon.py  # Automatically detects and uses ImageMagick
```

**Quality:** Excellent (uses native SVG rendering with Cairo backend)

---

### 2. **Inkscape** (High Quality) ⭐⭐⭐
Good alternative to ImageMagick.

**Installation:**
- Windows: Download from https://inkscape.org/release/
- After installation, add Inkscape to PATH or restart terminal

**Command:**
```bash
python convert-icon.py  # Automatically detects and uses Inkscape
```

**Quality:** Excellent (native SVG editor with export capabilities)

---

### 3. **cairosvg** (Good Quality) ⭐⭐
Python package using Cairo rendering library.

**Installation:**
```bash
pip install cairosvg
```

**Note:** Requires Cairo C library (libcairo-2.dll), which is difficult to install on Windows. Not recommended for Windows users.

**Quality:** Good (but hard to set up)

---

### 4. **PIL/Pillow** (Current Default) ⭐⭐
High-quality PIL rendering as fallback.

**Installation:**
```bash
pip install Pillow
```

**Features:**
- Creates icons at 4× resolution (1024×1024) then scales down with LANCZOS filtering
- Anti-aliased polygon rendering with transparency
- Includes shading, highlights, and improved geometry
- No external dependencies beyond Pillow

**Quality:** Good (clean, sharp edges at smaller sizes)

---

## Recommended Workflow

1. **First-time setup:**
   ```bash
   # Install Pillow for PIL rendering
   pip install Pillow
   
   # Generate initial icon
   python convert-icon-hq.py
   ```

2. **For production release (optional):**
   ```bash
   # Install ImageMagick from imagemagick.org
   # Then run:
   python convert-icon.py
   ```

3. **Edit the SVG:**
   - Open `prism-icon.svg` in Inkscape or Adobe Illustrator
   - Make changes
   - Save
   - Regenerate: `python convert-icon.py`

## Technical Details

### Icon Design
- **Concept:** 3D optical prism with light refraction
- **Colors:** 
  - Blue (0°-120°) - Input light
  - Red/Pink (120°-240°) - Refracted ray
  - Green (240°-360°) - Refracted ray
  - Yellow center ray
- **Effects:** Gradients, transparency, glow
- **Size:** 512×512 SVG base → 1024×1024 render → scaled to 6 sizes

### Building Executables

The build script (`../build.rs`) automatically:
1. Detects `assets/prism-icon.ico`
2. Embeds it into Windows executables using `winresource` crate
3. Adds version information metadata
4. Fails gracefully if icon is missing

To rebuild with icon:
```bash
cargo build --release --bin prism --bin prism-installer
```

## Troubleshooting

**Q: Icon looks pixelated in Windows Explorer**
- A: Use PIL rendering at higher resolution or install ImageMagick

**Q: "cairo-2 library not found" error**
- A: Normal on Windows. Remove cairosvg, use PIL instead: `pip uninstall cairosvg`

**Q: Executable doesn't show icon**
- A: Rebuild after icon generation: `cargo build --release`

**Q: Want to create custom colors**
- A: Edit `prism-icon.svg` directly or modify the RGB values in `convert-icon-hq.py`

## Format Specifications

- **Input:** SVG 512×512 (scalable)
- **Output:** ICO with multiple resolutions
- **Color Space:** RGBA (supports transparency)
- **Compression:** None (raw image data)
- **Windows Support:** Windows 7+
