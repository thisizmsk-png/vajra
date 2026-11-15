# App icons

The mark is an upright vajra: three prongs at each end, two collars and one hub.

Everything here except `src/` and the script is generated. Edit a drawing in `src/`, then run
`app/core/icons/build-icons.sh` (needs `rsvg-convert` and macOS `iconutil`) and commit the result.

| Source | Used for |
|---|---|
| `src/app-icon.svg` | The master, 1024 px canvas with an 824 px tile. Every size from 64 px up. |
| `src/app-icon-32.svg` | 32 px drawing on whole pixels, prongs open at the tip. |
| `src/app-icon-16.svg` | 16 px drawing, whole pixels only. |
| `src/menubar-mark.svg` | One-colour mark, for 32 px and up. |
| `src/menubar-mark-16.svg`, `src/menubar-mark-18.svg` | Pixel-aligned one-colour marks. |

| Output | Read by |
|---|---|
| `icon.icns`, `icon.png`, `32x32.png`, `128x128.png`, `128x128@2x.png` | `bundle.icon` in `tauri.conf.json` |
| `menubar/markTemplate-*.png` | Nothing yet. The app has no menu-bar item; these are ready for when it gets one. |

The menu-bar marks are template images: black with alpha only. Load them as a template so macOS
colours them for light and dark menu bars.

Colours: royal-blue tile `#2e5fe8` to `#16308f`, white prongs, gold `#f4c552` on the collars and hub.
The drawing does not depend on the palette, so a different colourway is a recolour of the three
`app-icon*.svg` files and one run of the script.
