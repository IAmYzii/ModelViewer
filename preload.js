const { contextBridge, ipcRenderer, webUtils } = require('electron');

contextBridge.exposeInMainWorld('viewerHost', {
  // Disk path of a dropped/opened file, so textures sitting next to the model
  // can be found automatically.
  pathForFile: file => webUtils.getPathForFile(file),
  // Watch one file for changes (null stops watching); onFileChanged fires when
  // it has been rewritten.
  watchFile: path => ipcRenderer.send('watch-file', path),
  onFileChanged: callback => ipcRenderer.on('file-changed', (_event, path) => callback(path)),
  // Put a PNG data URL on the clipboard as an image.
  copyImage: dataUrl => ipcRenderer.invoke('copy-image', dataUrl),
});
