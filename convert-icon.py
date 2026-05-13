#!/usr/bin/env python3
"""
Convert PRISM SVG icon to Windows .ico format
Uses alternative methods if cairosvg is not available
"""

import os
import sys
from pathlib import Path

def convert_svg_to_ico():
    try:
        from PIL import Image, ImageDraw
        import io
    except ImportError as e:
        print(f"Error: Missing required package: {e}")
        print("\nPlease install required packages:")
        print("  pip install Pillow")
        return False

    script_dir = Path(__file__).parent
    svg_file = script_dir / "assets" / "prism-icon.svg"
    ico_file = script_dir / "assets" / "prism-icon.ico"
    
    if not svg_file.exists():
        print(f"Error: {svg_file} not found")
        return False
    
    print(f"Creating {ico_file} from SVG...")
    
    try:
        # Try using cairosvg first
        try:
            import cairosvg
            png_data = io.BytesIO()
            cairosvg.svg2png(url=str(svg_file), write_to=png_data, output_width=256, output_height=256)
            png_data.seek(0)
            img = Image.open(png_data)
            print("✓ Used cairosvg for SVG conversion")
        except ImportError:
            print("Note: cairosvg not available, using PIL to create icon...")
            # Create icon programmatically if cairosvg not available
            img = create_prism_icon_pil()
        
        img = img.convert("RGBA")
        
        # Create icon with standard Windows sizes
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        icons = []
        
        for size in icon_sizes:
            resized = img.resize(size, Image.Resampling.LANCZOS)
            icons.append(resized)
        
        # Save as ICO
        icons[0].save(
            ico_file,
            format='ICO',
            sizes=[(size, size) for size in [16, 32, 48, 64, 128, 256]]
        )
        
        print(f"✓ Successfully created: {ico_file}")
        print(f"  Icon sizes: 16x16, 32x32, 48x48, 64x64, 128x128, 256x256")
        return True
        
    except Exception as e:
        print(f"Error during conversion: {e}")
        return False

def create_prism_icon_pil():
    """Create a PRISM icon programmatically using PIL"""
    from PIL import Image, ImageDraw
    
    # Create a 256x256 image with dark background
    img = Image.new('RGBA', (256, 256), (10, 14, 39, 255))
    draw = ImageDraw.Draw(img)
    
    # Draw a prism shape with gradients (simplified)
    # Front triangle (blue)
    points_front = [(128, 40), (70, 160), (186, 160)]
    draw.polygon(points_front, fill=(0, 212, 255, 255))
    
    # Left face (red/pink) 
    points_left = [(70, 160), (40, 190), (90, 230)]
    draw.polygon(points_left, fill=(255, 107, 157, 255))
    
    # Right face (green)
    points_right = [(186, 160), (216, 190), (166, 230)]
    draw.polygon(points_right, fill=(0, 255, 136, 255))
    
    # Add some highlights
    draw.ellipse([(110, 80), (140, 120)], fill=(255, 255, 255, 100))
    
    return img

if __name__ == "__main__":
    success = convert_svg_to_ico()
    sys.exit(0 if success else 1)

