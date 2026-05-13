from PIL import Image
from pathlib import Path
import os

def convert_svg_manually():
    script_dir = Path.cwd()
    svg_file = script_dir / 'assets' / 'prism-icon.svg'
    ico_file = script_dir / 'assets' / 'prism-icon.ico'
    
    print('Cairo is missing, attempting to use ImageMagick if available...')
    try:
        import subprocess
        # Try to use magick (ImageMagick 7+)
        subprocess.run(['magick', 'convert', '-background', 'none', str(svg_file), '-define', 'icon:auto-resize=16,32,48,64,128,256', str(ico_file)], check=True)
        print(f'✓ Successfully created using ImageMagick: {ico_file}')
        return True
    except Exception:
        print('ImageMagick not found or failed.')
        return False

if __name__ == '__main__':
    import sys
    success = convert_svg_manually()
    sys.exit(0 if success else 1)
