// Copies the release build into dist/: the prebuilt ModelViewer.exe and
// ModelViewerThumbnail.dll committed to the repo, so a clone installs without
// a Rust toolchain. Run through `npm run dist`, which builds first.
import { copyFileSync, existsSync, mkdirSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const target = process.env.CARGO_TARGET_DIR ?? join(root, 'src-tauri', 'target');
const out = join(root, 'dist');
mkdirSync(out, { recursive: true });

for (const name of ['ModelViewer.exe', 'ModelViewerThumbnail.dll']) {
  const from = join(target, 'release', name);
  if (!existsSync(from)) {
    console.error(`${from} is missing; run "npm run build" first.`);
    process.exit(1);
  }
  copyFileSync(from, join(out, name));
  console.log(`dist/${name}`);
}
