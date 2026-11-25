# Changelog

The running version shows in the bottom-right corner of the window as
`v<version> build <n> (<commit>)`: `<n>` is the number of commits the build was
made from, `<commit>` the short hash, with a `+` when the tree had uncommitted
changes. Bump the version in `package.json`, `src-tauri/tauri.conf.json` and
both `Cargo.toml` files together.

## 0.3.0 – 2026-11-25

### Added
- FBX 6.x binaries (what 3ds Max writes with the "FBX 6.1" preset) open:
  `fbx6.js` rewrites them as FBX 7 on the fly. Meshes, materials and textures
  carry over; animation does not.
- Explorer thumbnails for `.fbx` and `.obj`: the viewer saves a render on a
  file's first open and on the new Thumbnail button, and `ModelViewerThumbnail.dll`
  (`src-tauri/thumbnail`) serves it to the shell. `install.ps1` registers it.
- Version and build number in the corner of the window; this changelog.
- Prebuilt `ModelViewer.exe` and `ModelViewerThumbnail.dll` in `dist\`, so a
  clone installs with `install.ps1` alone; `npm run dist` refreshes them.

### Fixed
- Explorer and the shell's thumbnail cache pick up new and replaced thumbnails
  at once instead of keeping the old picture until the file changes.

## 0.2.0 – 2026-11-24

- Desktop shell ported from Electron to Tauri: one small exe on WebView2.
- Models open in tabs; files opened while the app runs join as tabs.
- 1 m grid cells; the reference cube snaps to them.

## 0.1.0 – 2026-11-24

- First version: FBX/OBJ viewer with shading modes, wireframe overlay, parts
  panel, selection, info panel, explode, reload on file change and screenshots.
