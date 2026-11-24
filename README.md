# Model Viewer

A quick viewer for `.fbx` and `.obj` files on Windows — a small replacement for
the discontinued 3D Viewer.

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
- View menu: floor grid (`G`), 1 m reference cube (`M`), file units, and a
  default rotation for files that have none remembered
- Frame model (`F`)
- Rotate 90° around X / Y / Z (`X`, `Y`, `Z`, Shift reverses) to fix badly
  oriented exports; the fix is remembered per file name

## Build and install

Requires Node.js.

```powershell
npm install
npm run build
.\install.ps1
```

`install.ps1` copies the app to `%LOCALAPPDATA%\Programs\ModelViewer` and adds
it to the "Open with" list for `.fbx` and `.obj` (current user, no admin).
`.\install.ps1 -Uninstall` removes it again.

## Develop

```powershell
npm start -- path\to\model.fbx
```

`viewer.html` also works on its own in a browser (it then loads three.js from a
CDN); drop a model onto the page.

`ModelViewer.exe model.fbx --screenshot=out.png` renders the model to a PNG and
exits. The view can be set from the command line:
`--shade=textured|simple|random|checker|vertex|faces|normals`, `--vc=rgb|r|g|b|a`,
`--wire=black|white|invert|custom`, `--explode=0..1`, `--select=<part index>`,
`--parts`, `--info`, `--cube`, `--normals`.
