const { app, BrowserWindow, ipcMain, dialog } = require('electron');
const path = require('path');
const fs = require('fs');
const crypto = require('crypto');
const { spawn } = require('child_process');

const userData = app.getPath('userData');
const versionFile = path.join(userData, 'version.json');
const gameDir = path.join(userData, 'game');

function readVersion() {
  try { return JSON.parse(fs.readFileSync(versionFile, 'utf8')); } catch { return { version: '0.0.0', files: [] }; }
}
function writeVersion(v) { fs.writeFileSync(versionFile, JSON.stringify(v, null, 2)); }

function ensureDir(d) { if (!fs.existsSync(d)) fs.mkdirSync(d, { recursive: true }); }

async function fetchManifest(channel) {
  const url = channel === 'dev'
    ? 'http://127.0.0.1:8080/manifest.json'
    : 'https://example.com/manifest.json';
  // fallback local stub
  const stubPath = path.join(__dirname, 'manifest.stub.json');
  if (fs.existsSync(stubPath)) {
    return JSON.parse(fs.readFileSync(stubPath, 'utf8'));
  }
  const res = await fetch(url);
  return await res.json();
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
  return { manifest, local, needsUpdate: manifest.version !== local.version };
});

ipcMain.handle('download-update', async (e, manifest) => {
  ensureDir(gameDir);
  const total = manifest.files.length;
  for (let i = 0; i < total; i++) {
    const f = manifest.files[i];
    const dest = path.join(gameDir, f.path);
    ensureDir(path.dirname(dest));
    // download
    const res = await fetch(f.url || `file://${path.join(__dirname, f.path)}`);
    // if url missing, skip (dev stub)
    if (!f.url) continue;
    const buf = Buffer.from(await res.arrayBuffer());
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
  const exe = path.join(gameDir, 'zn-client.exe');
  const args = [];
  if (opts && opts.token) args.push('--token', opts.token);
  if (opts && opts.scene) args.push('--scene', opts.scene);
  const child = spawn(exe, args, { detached: true, stdio: 'ignore' });
  child.unref();
  return { ok: true };
});

ipcMain.handle('get-local-version', () => readVersion());

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
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  if (process.platform !== 'darwin') app.quit();
});
