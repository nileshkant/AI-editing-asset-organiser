# SS-042 synthetic codec fixtures

No photographs, creator files or downloads are included. `generate.py` creates
solid RGB WebP (lossy VP8), two-frame lossless WebP and two-frame APNG from known
red/blue RGBA arrays. Generated with Pillow 11.3.0 and its libwebp runtime (version
printed by the script). Pillow is a fixture-generation tool only; the app has no
Python/Pillow runtime dependency. Regeneration need not produce byte-identical
compression across libwebp versions; dimensions/frame/pixel intent stay fixed.

The Rust integration suite independently generates JPEG, PNG (palette, grayscale,
RGBA, 8/16 bit), lossless WebP, EXIF orientations and matrix/TRC ICC profiles.
Known orientation maps, source alpha values and independently calculated color
patches are asserted against the real image-worker process. Corrupt/limit fixtures
are derived only from these synthetic bytes at test time.

Run `python -m venv /tmp/image-fixtures`, install `Pillow==11.3.0` in it, then run
this script with that environment's Python to regenerate the three tiny files.
