"""Generate tiny synthetic codec fixtures with Pillow 11.3.0 / native libwebp.
No downloaded photos or user media. Run in a disposable Python venv.
"""
from pathlib import Path
from PIL import Image, features
root = Path(__file__).resolve().parent
Image.new('RGB', (16, 8), (64, 128, 192)).save(root / 'solid-lossy.webp', quality=85)
a = Image.new('RGBA', (16, 16), (255, 0, 0, 255))
b = Image.new('RGBA', (16, 16), (0, 0, 255, 128))
a.save(root / 'two-frames.webp', save_all=True, append_images=[b], duration=500, loop=0, lossless=True)
a.save(root / 'two-frames.png', save_all=True, append_images=[b], duration=500, loop=0)
print('libwebp', features.version('webp'))
