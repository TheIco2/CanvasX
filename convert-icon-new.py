from PIL import Image
from pathlib import Path

def convert_png_to_ico():
    script_dir = Path.cwd()
    png_file = script_dir / 'assets' / 'prism-icon.png'
    ico_file = script_dir / 'assets' / 'prism-icon.ico'
    
    if not png_file.exists():
        print(f'Error: {png_file} not found')
        return False
    
    print(f'Converting {png_file} to .ico...')
    
    try:
        img = Image.open(png_file)
        img = img.convert('RGBA')
        
        icon_sizes = [(16, 16), (32, 32), (48, 48), (64, 64), (128, 128), (256, 256)]
        img.save(ico_file, format='ICO', sizes=icon_sizes)
        
        print(f'✓ Successfully created: {ico_file}')
        return True
    except Exception as e:
        print(f'Error during conversion: {e}')
        return False

if __name__ == '__main__':
    import sys
    success = convert_png_to_ico()
    sys.exit(0 if success else 1)
