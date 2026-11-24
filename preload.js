const { contextBridge, webUtils } = require('electron');

// Lets the page learn the disk path of a dropped/opened file, so textures
// sitting next to the model can be found automatically.
contextBridge.exposeInMainWorld('viewerHost', {
  pathForFile: file => webUtils.getPathForFile(file),
});
