# Model Viewer

A quick viewer for `.fbx` and `.obj` files on Windows — a small replacement for
the discontinued 3D Viewer.

- Tabs: every model opens in its own tab, including files opened from Explorer
  while the app is running (`Ctrl+Tab` switches, `Ctrl+W` or middle-click closes)
- Simple shading with a light that follows the camera
- Left drag rotates, right drag pans, wheel zooms toward the cursor
- Picks up textures and `.mtl` files next to the model (also from folders such
  as `textures`, `tex`, `maps`, `materials`)
- Shading modes (`1`–`7`): textured, simple, random color per part, UV checker,
  vertex colors (all or one channel), face orientation (back faces red), normals
- Wireframe overlay (`W`) in black, white, inverted surface color or a custom color
- Vertex-normal lines (`N`)
- Parts panel (`P`): show/hide parts, Alt+click to show only one; click a part
  in the view or the list to select it
- Info panel (`I`): size in real units, counts, which attributes exist, and
  details of the selected part
- Explode slider to pull parts apart
- Assign a texture to a part: drop an image on it, or use the Info panel
- Reloads the model when the file changes on disk, keeping the view (`F5` forces it)
- Screenshot with transparent background: `Ctrl+C` copies, `Ctrl+S` saves a PNG
- Thumbnails in Explorer (and any file manager that uses the shell's thumbnails):
  every model gets one the first time it is opened; the Thumbnail button replaces
  it with the current view. Grid and reference cube stay out of the picture
- View menu: floor grid (`G`), 1 m reference cube (`M`), file units, and a
  default rotation for files that have none remembered
- Frame model (`F`)
- Opens old FBX 6.x binaries (FBX 2006–2009, e.g. 3ds Max's "FBX 6.1" preset) by
  rewriting them as FBX 7 on the fly: meshes, materials and textures, no animation
- Rotate 90° around X / Y / Z (`X`, `Y`, `Z`, Shift reverses) to fix badly
  oriented exports; the fix is remembered per file name

## Build and install

The app is a [Tauri](https://tauri.app) shell (Rust) around `viewer.html`; it
uses the WebView2 runtime that ships with Windows 10/11, so the result is a
single small `ModelViewer.exe`.

Requires Node.js, Rust (rustup) and the Visual Studio C++ build tools.

```powershell
npm install
npm run build
.\install.ps1
```

`install.ps1` copies `ModelViewer.exe` and `ModelViewerThumbnail.dll` to
`%LOCALAPPDATA%\Programs\ModelViewer`, adds the viewer to the "Open with" list
for `.fbx` and `.obj` and registers the DLL as their thumbnail provider (current
user, no admin). `.\install.ps1 -Uninstall` removes it all again, thumbnails
included. Explorer loads the DLL in-process and lets go of it when idle, so a
newer copy takes over after a while; restarting Explorer makes it immediate.

If the build fails with `LNK1104: cannot open file`, the checkout path is too
long for Windows; set `CARGO_TARGET_DIR` to a short folder (`install.ps1`
honours it too).

## Develop

```powershell
npm run dev -- path	o\model.fbx
```

- `viewer.html` — the whole viewer (three.js). It also works on its own in a
  browser, loading three.js from a CDN; drop a model onto the page.
- `fbx6.js` — rewrites FBX 6.x binaries as FBX 7 ASCII text for three.js's
  `FBXLoader`, which only reads 7.x.
- `scripts/build-web.mjs` — copies `viewer.html`, `fbx6.js` and the three.js modules they
  import into `web/`, which is embedded into the exe.
- `src-tauri/src/main.rs` — serves disk files to the page, watches open files,
  handles "Open with", the clipboard, saving screenshots and the thumbnail cache
  (`%LOCALAPPDATA%\ModelViewer\thumbnails`, one PNG per file content hash).
- `src-tauri/thumbnail/` — `ModelViewerThumbnail.dll`, the COM thumbnail provider
  Explorer loads for `.fbx` and `.obj`; it only serves PNGs from that cache, so a
  file shows a thumbnail once it has been opened in the viewer.

Debug builds (`npm run dev`) can run next to an installed copy; release builds
hand their files to the window that is already open.

`ModelViewer.exe model.fbx --screenshot=out.png` renders the model to a PNG and
exits. The view can be set from the command line:
`--shade=textured|simple|random|checker|vertex|faces|normals`, `--vc=rgb|r|g|b|a`,
`--wire=black|white|invert|custom`, `--explode=0..1`, `--select=<part index>`,
`--parts`, `--info`, `--cube`, `--normals`, `--copy`.
