const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('zona', {
  checkUpdate: (channel) => ipcRenderer.invoke('check-update', channel),
  downloadUpdate: (manifest) => ipcRenderer.invoke('download-update', manifest),
  launchGame: (opts) => ipcRenderer.invoke('launch-game', opts),
  getLocalVersion: () => ipcRenderer.invoke('get-local-version'),
  onProgress: (cb) => ipcRenderer.on('download-progress', (_e, data) => cb(data)),
});
