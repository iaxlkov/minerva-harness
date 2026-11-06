# Minerva application artwork

`minerva-master.png` is the approved full-bleed portrait on a dark brown background.
`minerva-macos.png` uses a 1024px transparent canvas with an 824px rounded tile so its apparent size matches other Dock icons. The portrait remains bottom-aligned inside the tile.

Regenerate desktop and mobile sizes on macOS:

```sh
swift scripts/prepare-macos-icon.swift assets/minerva-master.png assets/minerva-macos.png
npx tauri icon assets/minerva-macos.png
```

The PNGs in `public/` use the full-bleed source for the browser favicon, touch icon, manifest, sidebar and empty-chat illustration. `src-tauri/icons/` contains the rounded native application icons, including ICNS and ICO.
