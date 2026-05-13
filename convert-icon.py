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
    """Enhanced PIL rendering as fallback"""
    try:
        from PIL import Image, ImageDraw
        
        print("Note: Using PIL rendering (lower quality fallback)")
        print("      For better quality, install ImageMagick or Inkscape")
        
        # Create high-resolution base image (512x512) then scale down
        img = Image.new('RGBA', (512, 512), (10, 14, 39, 255))
        draw = ImageDraw.Draw(img, 'RGBA')
        
        # Draw prism with better quality
        # Front face triangle (blue) with gradient simulation
        points_front = [(256, 80), (140, 320), (372, 320)]
        draw.polygon(points_front, fill=(0, 212, 255, 255))
        
        # Left face (red/pink)
        points_left = [(140, 320), (80, 380), (180, 460)]
        draw.polygon(points_left, fill=(255, 107, 157, 255))
        
        # Right face (green)
        points_right = [(372, 320), (432, 380), (332, 460)]
        draw.polygon(points_right, fill=(0, 255, 136, 255))
        
        # Bottom connecting face
        draw.polygon([(140, 320), (180, 460), (372, 320), (332, 460)], fill=(0, 168, 204, 150))
        
        # Light rays (input)
        for i in range(3):
            x_offset = i * 60 - 60
            draw.line([(256 + x_offset, 20), (228 + x_offset, 90)], fill=(0, 255, 255, 200), width=6)
        
        # Refracted light rays (output)
        draw.line([(140, 330), (80, 420)], fill=(255, 107, 157, 200), width=6)
        draw.line([(256, 330), (256, 420)], fill=(255, 255, 0, 180), width=4)
        draw.line([(372, 330), (432, 420)], fill=(0, 255, 136, 200), width=6)
        
        # Highlights
        draw.ellipse([(220, 140), (260, 220)], fill=(255, 255, 255, 80))
        
        # Now resize to standard sizes
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        icons = [img.resize(size, Image.Resampling.LANCZOS) for size in icon_sizes]
        
        # Save as ICO
        icons[0].save(ico_file, format='ICO', sizes=icon_sizes)
        
        print(f"✓ Successfully created using enhanced PIL: {ico_file}")
        print(f"  File size: {ico_file.stat().st_size} bytes")
        return True
    except Exception as e:
        print(f"Error in PIL rendering: {e}")
        return False

if __name__ == "__main__":
    success = convert_svg_to_ico()
    sys.exit(0 if success else 1)


