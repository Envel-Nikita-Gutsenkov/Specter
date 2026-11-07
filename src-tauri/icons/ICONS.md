# Icons

Place your application icons here before building.

Required files:
- `32x32.png`
- `128x128.png`
- `128x128@2x.png` (256x256 actual size)
- `icon.png` (used for tray icon, 32x32 or 64x64 recommended)
- `icon.ico` (Windows executable icon, multi-size ICO)

You can generate all icon sizes from a single high-resolution PNG (512x512 or 1024x1024)
using the Tauri CLI:

```
npx tauri icon path/to/your/icon.png
```

This command automatically creates all required sizes and formats in this directory.
