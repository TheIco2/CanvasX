#!/usr/bin/env python3
"""
Convert PRISM SVG icon to Windows .ico format
Tries multiple methods for best quality:
1. ImageMagick (convert/magick command)
2. Inkscape
3. cairosvg
4. PIL with high-quality rendering
"""

import os
import sys
import subprocess
from pathlib import Path

def convert_svg_to_ico():
    script_dir = Path(__file__).parent
    svg_file = script_dir / "assets" / "prism-icon.svg"
    ico_file = script_dir / "assets" / "prism-icon.ico"
    
    if not svg_file.exists():
        print(f"Error: {svg_file} not found")
        return False
    
    print(f"Converting {svg_file} to .ico...\n")
    
    # Try ImageMagick first (best quality)
    if try_imagemagick(svg_file, ico_file):
        return True
    
    # Try Inkscape
    if try_inkscape(svg_file, ico_file):
        return True
    
    # Try cairosvg
    if try_cairosvg(svg_file, ico_file):
        return True
    
    # Fall back to enhanced PIL
    if try_pil_enhanced(svg_file, ico_file):
        return True
    
    print("Error: No suitable SVG converter found!")
    print("\nTo get better quality icons, install one of:")
    print("  - ImageMagick (https://imagemagick.org/script/download.php)")
    print("  - Inkscape (https://inkscape.org/release/)")
    print("  - pip install cairosvg (requires Cairo C library)")
    return False

def try_imagemagick(svg_file, ico_file):
    """Try using ImageMagick convert or magick command"""
    for cmd in ["convert", "magick convert"]:
        try:
            # Create intermediate PNG at high resolution for better quality
            png_file = svg_file.parent / "prism-icon-temp.png"
            subprocess.run(
                f'{cmd} "{svg_file}" -background none -density 300 -resize 256x256 "{png_file}"',
                shell=True,
                check=True,
                capture_output=True,
                timeout=10
            )
            
            # Convert PNG to ICO
            subprocess.run(
                f'{cmd} "{png_file}" -define icon:auto-resize=256,128,96,64,48,32,16 "{ico_file}"',
                shell=True,
                check=True,
                capture_output=True,
                timeout=10
            )
            
            png_file.unlink()
            print(f"✓ Successfully created using ImageMagick: {ico_file}")
            return True
        except (subprocess.CalledProcessError, FileNotFoundError, subprocess.TimeoutExpired):
            continue
    
    return False

def try_inkscape(svg_file, ico_file):
    """Try using Inkscape"""
    try:
        # Inkscape can export to PNG
        png_file = svg_file.parent / "prism-icon-temp.png"
        subprocess.run(
            f'inkscape --export-filename="{png_file}" --export-width=256 --export-height=256 "{svg_file}"',
            shell=True,
            check=True,
            capture_output=True,
            timeout=10
        )
        
        # Convert PNG to ICO using PIL
        from PIL import Image
        img = Image.open(png_file)
        img = img.convert("RGBA")
        
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        icons = [img.resize(size, Image.Resampling.LANCZOS) for size in icon_sizes]
        
        icons[0].save(ico_file, format='ICO', sizes=icon_sizes)
        png_file.unlink()
        print(f"✓ Successfully created using Inkscape + PIL: {ico_file}")
        return True
    except (subprocess.CalledProcessError, FileNotFoundError, subprocess.TimeoutExpired):
        return False
    except Exception:
        return False

def try_cairosvg(svg_file, ico_file):
    """Try using cairosvg"""
    try:
        import cairosvg
        from PIL import Image
        import io
        
        # Convert SVG to PNG at high resolution
        png_data = io.BytesIO()
        cairosvg.svg2png(url=str(svg_file), write_to=png_data, output_width=512, output_height=512)
        png_data.seek(0)
        
        img = Image.open(png_data)
        img = img.convert("RGBA")
        
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        icons = [img.resize(size, Image.Resampling.LANCZOS) for size in icon_sizes]
        
        icons[0].save(ico_file, format='ICO', sizes=icon_sizes)
        print(f"✓ Successfully created using cairosvg: {ico_file}")
        return True
    except ImportError:
        return False
    except Exception:
        return False

def try_pil_enhanced(svg_file, ico_file):
    """Enhanced PIL rendering as fallback (calls the HQ script)"""
    try:
        # Import and run the HQ generator
        import convert_icon_hq
        return convert_icon_hq.main()
    except Exception:
        # Fall back to inline version if import fails
        return try_pil_inline(svg_file, ico_file)

def try_pil_inline(svg_file, ico_file):
    """Inline PIL rendering fallback"""
    try:
        from PIL import Image, ImageDraw
        
        print("Note: Using PIL rendering (high-quality fallback)")
        print("      For best quality, install ImageMagick or Inkscape")
        
        # Create high-resolution base image (1024x1024) then scale down
        img = Image.new('RGBA', (1024, 1024), (10, 14, 39, 255))
        draw = ImageDraw.Draw(img, 'RGBA')
        
        scale = 2  # 512 design scaled to 1024
        
        def s(x, y):
            return int(x * scale), int(y * scale)
        
        # Front face (blue)
        draw.polygon([s(256, 80), s(140, 320), s(372, 320)], fill=(0, 212, 255, 255))
        draw.polygon([s(256, 80), s(200, 180), s(312, 180)], fill=(0, 240, 255, 100))
        
        # Left face (red/pink)
        draw.polygon([s(140, 320), s(80, 380), s(180, 460)], fill=(255, 107, 157, 255))
        draw.polygon([s(140, 320), s(110, 350), s(130, 390)], fill=(200, 70, 120, 150))
        
        # Right face (green)
        draw.polygon([s(372, 320), s(432, 380), s(332, 460)], fill=(0, 255, 136, 255))
        draw.polygon([s(372, 320), s(402, 350), s(382, 390)], fill=(0, 200, 100, 150))
        
        # Bottom face (shadow)
        draw.polygon([s(140, 320), s(180, 460), s(372, 320), s(332, 460)], fill=(0, 168, 204, 120))
        
        # Light rays
        draw.line([s(256, 20), s(256, 80)], fill=(255, 255, 255, 220), width=24)
        draw.line([s(200, 30), s(180, 100)], fill=(0, 200, 255, 180), width=16)
        draw.line([s(312, 30), s(332, 100)], fill=(0, 200, 255, 180), width=16)
        
        # Refracted rays
        draw.line([s(140, 330), s(80, 420)], fill=(255, 107, 157, 220), width=24)
        draw.line([s(256, 330), s(256, 420)], fill=(255, 255, 0, 200), width=20)
        draw.line([s(372, 330), s(432, 420)], fill=(0, 255, 136, 220), width=24)
        
        # Highlights
        draw.polygon([s(220, 120), s(240, 100), s(260, 110), s(250, 140), s(230, 130)],
                    fill=(255, 255, 255, 120))
        
        # Resize and save
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        icons = [img.resize(size, Image.Resampling.LANCZOS) for size in icon_sizes]
        
        icons[0].save(ico_file, format='ICO', sizes=icon_sizes)
        
        print(f"✓ Successfully created using high-quality PIL: {ico_file}")
        print(f"  File size: {ico_file.stat().st_size} bytes")
        return True
    except Exception as e:
        print(f"Error in PIL rendering: {e}")
        return False

if __name__ == "__main__":
    success = convert_svg_to_ico()
    sys.exit(0 if success else 1)


