if (process.env.ELECTRON_RUN_AS_NODE) {
  console.error('ELECTRON_RUN_AS_NODE is set. Please unset it before running ZONA Launcher.');
  process.exit(1);
}
const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const path = require('path');
const fs = require('fs');
const crypto = require('crypto');
const { spawn } = require('child_process');
const { autoUpdater } = require('electron-updater');

const userData = app.getPath('userData');
const settingsFile = path.join(userData, 'settings.json');
const defaultGameDir = path.join(userData, 'game');
const logFile = path.join(userData, 'launcher.log');
function log(msg) {
  try { fs.appendFileSync(logFile, new Date().toISOString() + ' ' + msg + '\n'); } catch {}
}

function readSettings() {
  try { return JSON.parse(fs.readFileSync(settingsFile, 'utf8')); } catch { return {}; }
}
function writeSettings(s) { fs.writeFileSync(settingsFile, JSON.stringify(s, null, 2)); }
function getGameDir() {
  const s = readSettings();
  if (s.installDir && fs.existsSync(s.installDir)) return s.installDir;
  return defaultGameDir;
}
function setGameDir(newDir) {
  const oldDir = getGameDir();
  fs.mkdirSync(newDir, { recursive: true });
  // migrate files
  for (const f of ['zn-client.exe', 'version.json']) {
    const src = path.join(oldDir, f);
    const dst = path.join(newDir, f);
    if (fs.existsSync(src) && !fs.existsSync(dst)) {
      try { fs.renameSync(src, dst); } catch { fs.copyFileSync(src, dst); try { fs.unlinkSync(src); } catch {} }
    }
  }
  const s = readSettings();
  s.installDir = newDir;
  writeSettings(s);
  return newDir;
}
const versionFile = () => path.join(getGameDir(), 'version.json');
const gameDir = () => getGameDir();

function readVersion() {
  try { return JSON.parse(fs.readFileSync(versionFile(), 'utf8')); } catch { return { version: '0.0.0', files: [] }; }
}
function writeVersion(v) { fs.writeFileSync(versionFile(), JSON.stringify(v, null, 2)); }

function ensureDir(d) { if (!fs.existsSync(d)) fs.mkdirSync(d, { recursive: true }); }

async function fetchManifest(channel) {
  const url = 'https://github.com/Otakyooo/zona_engine/releases/latest/download/manifest.json';
  const stubPath = path.join(__dirname, 'manifest.stub.json');
  try {
    const res = await fetch(url);
    if (res.ok) return await res.json();
    throw new Error('HTTP ' + res.status);
  } catch (e) {
    log('fetchManifest error: ' + e.message);
    if (fs.existsSync(stubPath)) {
      try { return JSON.parse(fs.readFileSync(stubPath, 'utf8')); } catch {}
    }
    return { version: '0.0.0', files: [], error: e.message };
  }
}

function sha256File(filePath) {
  return new Promise((resolve, reject) => {
    const hash = crypto.createHash('sha256');
    const stream = fs.createReadStream(filePath);
    stream.on('data', (d) => hash.update(d));
    stream.on('end', () => resolve(hash.digest('hex')));
    stream.on('error', reject);
  });
}

ipcMain.handle('check-update', async (_e, channel) => {
  const manifest = await fetchManifest(channel);
  const local = readVersion();
  const exe = path.join(getGameDir(), 'zn-client.exe');
  const exeMissing = !fs.existsSync(exe);
  let needsUpdate = manifest.version !== local.version;
  if (manifest.files && manifest.files.length === 0) {
    needsUpdate = exeMissing;
  }
  if (manifest.error) {
    return { manifest, local, needsUpdate, error: manifest.error };
  }
  return { manifest, local, needsUpdate };
});

ipcMain.handle('download-update', async (e, manifest) => {
  ensureDir(getGameDir());
  const total = manifest.files.length;
  for (let i = 0; i < total; i++) {
    const f = manifest.files[i];
    const dest = path.join(getGameDir(), f.path);
    ensureDir(path.dirname(dest));
    // download
    let buf;
    if (f.url && f.url.startsWith('file://')) {
      const filePath = decodeURIComponent(f.url.replace('file:///', '').replace('file://', ''));
      buf = fs.readFileSync(filePath);
    } else if (f.url) {
      const res = await fetch(f.url);
      buf = Buffer.from(await res.arrayBuffer());
    } else {
      continue;
    }
    fs.writeFileSync(dest + '.tmp', buf);
    // sha256
    const hash = crypto.createHash('sha256').update(buf).digest('hex');
    if (hash !== f.sha256) {
      fs.unlinkSync(dest + '.tmp');
      throw new Error(`SHA mismatch for ${f.path}`);
    }
    // zstd unpack if .zst
    if (dest.endsWith('.zst')) {
      const { decompress } = require('fzstd');
      const packed = fs.readFileSync(dest + '.tmp');
      const out = decompress(packed);
      fs.writeFileSync(dest.replace(/\.zst$/, ''), Buffer.from(out));
      fs.unlinkSync(dest + '.tmp');
    } else {
      fs.renameSync(dest + '.tmp', dest);
    }
    e.sender.send('download-progress', { current: i + 1, total, file: f.path });
  }
  writeVersion({ version: manifest.version, files: manifest.files.map((f) => f.path) });
  return { ok: true };
});

ipcMain.handle('launch-game', async (_e, opts) => {
  const exe = path.join(getGameDir(), 'zn-client.exe');
  if (!fs.existsSync(exe)) {
    log('launch-game: exe missing ' + exe);
    return { ok: false, error: 'Game not installed. Click Update first.' };
  }
  const args = [];
  if (opts && opts.server) { args.push('--server', opts.server); }
  if (opts && opts.port) { args.push('--port', String(opts.port)); }
  if (opts && opts.token) args.push('--token', opts.token);
  if (opts && opts.scene) args.push('--scene', opts.scene);
  const child = spawn(exe, args, { detached: true, stdio: 'ignore' });
  child.unref();
  return { ok: true };
});

ipcMain.handle('get-local-version', () => readVersion());
ipcMain.handle('get-install-dir', async () => getGameDir());
ipcMain.handle('pick-install-dir', async () => {
  const win = BrowserWindow.getFocusedWindow();
  const res = await dialog.showOpenDialog(win, { properties: ['openDirectory'] });
  if (res.canceled || !res.filePaths.length) return null;
  return res.filePaths[0];
});
ipcMain.handle('set-install-dir', async (_e, newDir) => {
  if (!newDir) return { ok: false, error: 'no path' };
  const p = setGameDir(newDir);
  return { ok: true, path: p };
});

function createWindow() {
  const win = new BrowserWindow({
    width: 960,
    height: 600,
    webPreferences: { preload: path.join(__dirname, 'preload.js'), contextIsolation: true },
  });
  win.loadFile('index.html');
}

app.whenReady().then(() => {
  createWindow();
  // self-update
  try {
    autoUpdater.autoDownload = true;
    autoUpdater.allowPrerelease = true;
    autoUpdater.verifyUpdateCodeSignature = false;
    if (!app.isPackaged) autoUpdater.forceDevUpdateConfig = true;
    autoUpdater.checkForUpdatesAndNotify();
  } catch (e) {
    console.error('autoUpdater error', e);
  }
  autoUpdater.on('update-downloaded', () => {
    autoUpdater.quitAndInstall();
  });
  autoUpdater.on('error', (e) => console.error('autoUpdater', e));
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
