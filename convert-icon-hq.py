#!/usr/bin/env python3
"""
High-quality PRISM icon converter
Generates crisp, high-resolution icons using multiple methods
"""

import sys
from pathlib import Path

def main():
    from PIL import Image, ImageDraw
    
    script_dir = Path(__file__).parent
    svg_file = script_dir / "assets" / "prism-icon.svg"
    ico_file = script_dir / "assets" / "prism-icon.ico"
    
    if not svg_file.exists():
        print(f"Error: {svg_file} not found")
        return False
    
    print("Creating high-quality PRISM icon...\n")
    
    # Create at 4x resolution for better quality when scaled down
    base_size = 1024
    img = Image.new('RGBA', (base_size, base_size), (10, 14, 39, 255))
    draw = ImageDraw.Draw(img)
    
    scale = base_size / 512  # Scale factor from 512 design to our render size
    
    # Helper to scale coordinates
    def s(x, y):
        return int(x * scale), int(y * scale)
    
    # Draw prism with anti-aliased polygons at high resolution
    
    # Front face triangle (blue gradient effect)
    points_front = [s(256, 80), s(140, 320), s(372, 320)]
    draw.polygon(points_front, fill=(0, 212, 255, 255))
    
    # Add subtle shading to front face
    draw.polygon([s(256, 80), s(200, 180), s(312, 180)], fill=(0, 240, 255, 100))
    
    # Left face (red/pink)
    points_left = [s(140, 320), s(80, 380), s(180, 460)]
    draw.polygon(points_left, fill=(255, 107, 157, 255))
    
    # Left face darker edge
    draw.polygon([s(140, 320), s(110, 350), s(130, 390)], fill=(200, 70, 120, 150))
    
    # Right face (green)
    points_right = [s(372, 320), s(432, 380), s(332, 460)]
    draw.polygon(points_right, fill=(0, 255, 136, 255))
    
    # Right face darker edge
    draw.polygon([s(372, 320), s(402, 350), s(382, 390)], fill=(0, 200, 100, 150))
    
    # Bottom connecting face (shadow)
    draw.polygon([s(140, 320), s(180, 460), s(372, 320), s(332, 460)], fill=(0, 168, 204, 120))
    
    # Draw light rays (incoming white light)
    ray_width = int(12 * scale)
    
    # Center incoming ray (white)
    draw.line([s(256, 20), s(256, 80)], fill=(255, 255, 255, 220), width=ray_width)
    
    # Side rays (cyan tint)
    draw.line([s(200, 30), s(180, 100)], fill=(0, 200, 255, 180), width=int(8 * scale))
    draw.line([s(312, 30), s(332, 100)], fill=(0, 200, 255, 180), width=int(8 * scale))
    
    # Refracted light rays (colored output)
    
    # Red/Pink ray (left)
    draw.line([s(140, 330), s(80, 420)], fill=(255, 107, 157, 220), width=ray_width)
    
    # Yellow ray (center)
    draw.line([s(256, 330), s(256, 420)], fill=(255, 255, 0, 200), width=int(10 * scale))
    
    # Green ray (right)
    draw.line([s(372, 330), s(432, 420)], fill=(0, 255, 136, 220), width=ray_width)
    
    # Add highlight/shine on prism
    highlight_points = [
        s(220, 120), s(240, 100), s(260, 110),
        s(250, 140), s(230, 130)
    ]
    draw.polygon(highlight_points, fill=(255, 255, 255, 120))
    
    # Add glow effect around prism
    for i in range(3, 0, -1):
        alpha = int(40 / i)
        draw.polygon([s(256, 80), s(140, 320), s(372, 320)], 
                    outline=(0, 212, 255, alpha), width=int(i * scale))
    
    # Now create the final icon with proper sizes
    icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
    
    print("Generating icon at multiple resolutions:")
    icons = []
    for size in icon_sizes:
        # Use high-quality downsampling
        resized = img.resize(size, Image.Resampling.LANCZOS)
        icons.append(resized)
        print(f"  ✓ {size[0]}x{size[1]}")
    
    # Save as ICO
    icons[0].save(ico_file, format='ICO', sizes=icon_sizes)
    
    file_size = ico_file.stat().st_size
    print(f"\n✓ Created: {ico_file}")
    print(f"  File size: {file_size} bytes")
    print(f"  Quality: High (rendered at {base_size}x{base_size})")
    
    return True

if __name__ == "__main__":
    try:
        success = main()
        sys.exit(0 if success else 1)
    except Exception as e:
        print(f"Error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)
