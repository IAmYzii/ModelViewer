const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('viewerHost', {
  // Disk path of a dropped/opened file, so textures sitting next to the model
  // can be found automatically.
  pathForFile: file => webUtils.getPathForFile(file),
  // Watch this list of files for changes (replacing the previous list);
  // onFileChanged fires when one of them has been rewritten.
  watchFiles: paths => ipcRenderer.send('watch-files', paths),
  onFileChanged: callback => ipcRenderer.on('file-changed', (_event, path) => callback(path)),
  // Models to open in new tabs, sent when the app is started again with files.
  onOpenFiles: callback => ipcRenderer.on('open-files', (_event, paths) => callback(paths)),
  // Put a PNG data URL on the clipboard as an image.
  copyImage: dataUrl => ipcRenderer.invoke('copy-image', dataUrl),
});
