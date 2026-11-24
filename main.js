const { app, BrowserWindow, protocol, Menu, ipcMain, clipboard, ClipboardItem } = require('electron');
const path = require('node:path');
const fs = require('node:fs');

// Everything is served from app://app/ so the page, the bundled three.js and
// the model files on disk share one origin:
//   app://app/viewer.html            the viewer page
//   app://app/node_modules/three/…   bundled three.js
//   app://app/@fs/C:/dir/model.fbx   any file on disk (model, .mtl, textures)
protocol.registerSchemesAsPrivileged([
  { scheme: 'app', privileges: { standard: true, secure: true, supportFetchAPI: true } },
]);

const MIME = {
  html: 'text/html; charset=utf-8', js: 'text/javascript', json: 'application/json',
  png: 'image/png', jpg: 'image/jpeg', jpeg: 'image/jpeg', gif: 'image/gif',
  bmp: 'image/bmp', webp: 'image/webp',
};
const CDN = 'https://cdn.jsdelivr.net/npm/three@0.170.0/';
const TEXTURE_DIRS = ['textures', 'texture', 'tex', 'maps', 'materials', 'images',
  '../textures', '../texture', '../tex', '../maps', '../materials', '../images', '..'];

async function exists(p) {
  try { return (await fs.promises.stat(p)).isFile(); } catch { return false; }
}

// A file referenced by a model. If it is not where the model says, look in the
// usual texture folders around it.
async function resolveDiskFile(p) {
  if (await exists(p)) return p;
  const dir = path.dirname(p), name = path.basename(p);
  for (const sub of TEXTURE_DIRS) {
    const candidate = path.join(dir, sub, name);
    if (await exists(candidate)) return candidate;
  }
  return null;
}

async function respondWithFile(file) {
  const ext = path.extname(file).slice(1).toLowerCase();
  const data = await fs.promises.readFile(file);
  // no-store: a model that is re-exported must not come back from a cache.
  return new Response(data, {
    headers: { 'content-type': MIME[ext] ?? 'application/octet-stream', 'cache-control': 'no-store' },
  });
}

async function handleRequest(request) {
  const pathname = decodeURIComponent(new URL(request.url).pathname);
  try {
    if (pathname.startsWith('/@fs/')) {
      const file = await resolveDiskFile(path.normalize(pathname.slice(5)));
      return file ? respondWithFile(file) : new Response('Not found', { status: 404 });
    }
    if (pathname === '/viewer.html') {
      // viewer.html also works standalone in a browser, where it pulls three.js
      // from a CDN. Inside the app, point it at the bundled copy instead.
      const html = (await fs.promises.readFile(path.join(__dirname, 'viewer.html'), 'utf8'))
        .replaceAll(CDN, '/node_modules/three/');
      return new Response(html, { headers: { 'content-type': MIME.html } });
    }
    const threeRoot = path.join(__dirname, 'node_modules', 'three');
    const file = path.join(__dirname, pathname);
    if (file.startsWith(threeRoot + path.sep)) return respondWithFile(file);
  } catch (e) {
    console.error(e);
  }
  return new Response('Not found', { status: 404 });
}

const diskUrl = p => '/@fs/' + p.replace(/\\/g, '/').split('/').map(encodeURIComponent).join('/');

function modelsFromArgs(argv, cwd) {
  return argv.slice(1)
    .filter(arg => /\.(fbx|obj)$/i.test(arg))
    .map(arg => path.resolve(cwd, arg))
    .filter(p => fs.existsSync(p));
}

function argValue(argv, name) {
  const hit = argv.find(a => a.startsWith(`--${name}=`));
  return hit ? hit.slice(name.length + 3) : null;
}

// A window watches the files of its tabs and is told when one was rewritten.
const watchers = new Map();   // webContents id -> Map(file -> listener)
function unwatch(id, keep = []) {
  const files = watchers.get(id);
  if (!files) return;
  for (const [file, listener] of files) {
    if (keep.includes(file)) continue;
    fs.unwatchFile(file, listener);
    files.delete(file);
  }
  if (!files.size) watchers.delete(id);
}
ipcMain.on('watch-files', (event, list) => {
  const sender = event.sender, id = sender.id;
  const wanted = Array.isArray(list) ? list.filter(f => typeof f === 'string' && f) : [];
  unwatch(id, wanted);
  const files = watchers.get(id) ?? new Map();
  for (const file of wanted) {
    if (files.has(file)) continue;
    let timer = null;
    const listener = (cur, prev) => {
      if (!cur.size || (cur.mtimeMs === prev.mtimeMs && cur.size === prev.size)) return;
      // Exporters write in several steps; wait until the file has been quiet.
      clearTimeout(timer);
      timer = setTimeout(() => { if (!sender.isDestroyed()) sender.send('file-changed', file); }, 400);
    };
    fs.watchFile(file, { interval: 500 }, listener);
    files.set(file, listener);
  }
  if (files.size) watchers.set(id, files);
});
// Returns nothing on success; a failure rejects in the page, which reports it.
ipcMain.handle('copy-image', async (_event, dataUrl) => {
  if (typeof dataUrl !== 'string' || !dataUrl.startsWith('data:image/png;base64,')) {
    throw new Error('Not a PNG data URL');
  }
  const png = Buffer.from(dataUrl.slice(dataUrl.indexOf(',') + 1), 'base64');
  await clipboard.write([new ClipboardItem({ 'image/png': new Blob([png], { type: 'image/png' }) })]);
});

let mainWindow = null;

function createWindow(models) {
  const win = new BrowserWindow({
    width: 1280, height: 800, backgroundColor: '#1c1e22', title: 'Model Viewer',
    webPreferences: { preload: path.join(__dirname, 'preload.js') },
  });
  win.webContents.on('before-input-event', (_e, input) => {
    if (input.type === 'keyDown' && input.key === 'F12') win.webContents.toggleDevTools();
  });
  // Dropping a non-model file must not navigate the window away from the viewer.
  win.webContents.on('will-navigate', e => e.preventDefault());
  win.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  const contentsId = win.webContents.id;
  win.webContents.on('destroyed', () => unwatch(contentsId));
  // Switches that set up the view, see README.
  const params = new URLSearchParams();
  for (const model of models) params.append('src', diskUrl(model));
  for (const name of ['shade', 'wire', 'vc', 'explode', 'select']) {
    const value = argValue(process.argv, name);
    if (value) params.set(name, value);
  }
  for (const flag of ['parts', 'info', 'cube', 'normals', 'copy']) {
    if (process.argv.includes('--' + flag)) params.set(flag, '1');
  }
  win.loadURL('app://app/viewer.html?' + params);
  win.on('closed', () => { if (mainWindow === win) mainWindow = null; });
  mainWindow = win;
  return win;
}

// A development run (npm start) keeps its own profile, so it neither hands its
// file over to an installed Model Viewer that happens to be open nor shares its
// saved settings.
if (!app.isPackaged) app.setPath('userData', path.join(app.getPath('appData'), 'Model Viewer (dev)'));

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  // Started again (for example "Open with" in Explorer): the files become new
  // tabs in the window that is already open.
  app.on('second-instance', (_e, argv, cwd) => {
    const models = modelsFromArgs(argv, cwd);
    if (!mainWindow) { createWindow(models); return; }
    if (models.length) mainWindow.webContents.send('open-files', models);
    if (mainWindow.isMinimized()) mainWindow.restore();
    mainWindow.show();
    mainWindow.focus();
  });
  app.on('window-all-closed', () => app.quit());

  app.whenReady().then(() => {
    Menu.setApplicationMenu(null);
    protocol.handle('app', handleRequest);
    const win = createWindow(modelsFromArgs(process.argv, process.cwd()));

    // ModelViewer.exe model.fbx --screenshot=out.png  → save a picture and exit.
    const shot = argValue(process.argv, 'screenshot');
    if (shot) {
      let done = false;
      const capture = async () => {
        if (done) return;
        done = true;
        // VIEWER_SHOT_DELAY (ms) postpones the capture, for scripted checks.
        await new Promise(r => setTimeout(r, Number(process.env.VIEWER_SHOT_DELAY) || 1500));
        const image = await win.webContents.capturePage();
        fs.writeFileSync(path.resolve(shot), image.toPNG());
        app.quit();
      };
      win.on('page-title-updated', (_e, title) => { if (title.includes('–')) capture(); });
      setTimeout(capture, 20000);
    }
  });
}
