const { contextBridge, ipcRenderer } = require('electron');
contextBridge.exposeInMainWorld('zona', {
  checkUpdate: (channel) => ipcRenderer.invoke('check-update', channel),
  downloadUpdate: (manifest) => ipcRenderer.invoke('download-update', manifest),
  launchGame: (opts) => ipcRenderer.invoke('launch-game', opts),
  getLocalVersion: () => ipcRenderer.invoke('get-local-version'),
  getInstallDir: () => ipcRenderer.invoke('get-install-dir'),
  pickInstallDir: () => ipcRenderer.invoke('pick-install-dir'),
  setInstallDir: (p) => ipcRenderer.invoke('set-install-dir', p),
  onProgress: (cb) => ipcRenderer.on('download-progress', (_e, data) => cb(data)),
});
