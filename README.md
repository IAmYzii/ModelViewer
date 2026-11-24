# Model Viewer

A quick viewer for `.fbx` and `.obj` files on Windows — a small replacement for
the discontinued 3D Viewer.

- Simple shading with a light that follows the camera
- Left drag rotates, right drag pans, wheel zooms toward the cursor
- Picks up textures and `.mtl` files next to the model (also from folders such
  as `textures`, `tex`, `maps`, `materials`)
- Wireframe (`W`), floor grid (`G`), frame model (`F`)
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
exits.
