// Assembles the folder that gets embedded into the app (web/):
//   viewer.html, pointed at a local copy of three.js instead of the CDN
//   three/…      only the three.js modules the viewer actually imports
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync, existsSync } from 'node:fs';
import { execSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const three = join(root, 'node_modules', 'three');
const out = join(root, 'web');
const CDN = /https:\/\/cdn\.jsdelivr\.net\/npm\/three@[\d.]+\//g;

if (!existsSync(three)) {
  console.error('three.js is missing. Run "npm install" first.');
  process.exit(1);
}

rmSync(out, { recursive: true, force: true });
mkdirSync(out, { recursive: true });

// "0.3.0 build 12 (7c0cfb1)"; a trailing + marks uncommitted changes. Shown in the window corner.
const git = args => { try { return execSync(`git ${args}`, { cwd: root, stdio: ['ignore', 'pipe', 'ignore'] }).toString().trim(); } catch { return ''; } };
const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'));
const dirty = git('status --porcelain -- . ":!dist"') ? '+' : '';
const built = new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC';
const build = `${version} build ${git('rev-list --count HEAD') || '?'} (${git('rev-parse --short HEAD') || 'no git'}${dirty}) ${built}`;

const html = readFileSync(join(root, 'viewer.html'), 'utf8');
writeFileSync(join(out, 'viewer.html'), html.replace(CDN, './three/').replace('__VERSION__', build));
const fbx6 = readFileSync(join(root, 'fbx6.js'), 'utf8');
writeFileSync(join(out, 'fbx6.js'), fbx6);

// Start from the modules viewer.html and fbx6.js import and follow their relative imports.
const queue = ['build/three.module.js'];
for (const m of (html + fbx6).matchAll(/from 'three\/addons\/([^']+)'/g)) queue.push('examples/jsm/' + m[1]);
const seen = new Set();
while (queue.length) {
  const file = queue.pop();
  if (seen.has(file)) continue;
  seen.add(file);
  const source = join(three, file);
  const target = join(out, 'three', file);
  mkdirSync(dirname(target), { recursive: true });
  cpSync(source, target);
  const code = readFileSync(source, 'utf8');
  for (const m of code.matchAll(/(?:from|import)\s*['"](\.{1,2}\/[^'"]+)['"]/g)) {
    queue.push(join(dirname(file), m[1]).replace(/\\/g, '/'));
  }
}
console.log(`web/: viewer.html + ${seen.size} three.js modules, version ${build}`);
