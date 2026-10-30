const channelSel = document.getElementById('channel');
const statusEl = document.getElementById('status');
const newsEl = document.getElementById('news');
const playBtn = document.getElementById('play');
const progressBar = document.getElementById('progress-bar');
const progressText = document.getElementById('progress-text');
const versionEl = document.getElementById('version');
const localVersionEl = document.getElementById('local-version');
const serverInput = document.getElementById('server');
serverInput.value = localStorage.getItem('server') || '192.168.0.255';
serverInput.addEventListener('change', () => localStorage.setItem('server', serverInput.value));

channelSel.value = localStorage.getItem('channel') || 'stable';
channelSel.addEventListener('change', () => localStorage.setItem('channel', channelSel.value));

window.zona.onProgress((data) => {
  const pct = Math.round((data.current / data.total) * 100);
  progressBar.style.width = pct + '%';
  progressText.textContent = `${data.current}/${data.total} ${data.file}`;
});

async function init() {
  const local = await window.zona.getLocalVersion();
  localVersionEl.textContent = 'local: ' + (local.version || 'none');
  newsEl.textContent = 'Welcome to ZONA. Checking for updates...';
  // auto-check updates
  try {
    const channel = channelSel.value;
    const { manifest, needsUpdate } = await window.zona.checkUpdate(channel);
    versionEl.textContent = 'v' + manifest.version;
    if (needsUpdate) {
      statusEl.textContent = 'Downloading update...';
      await window.zona.downloadUpdate(manifest);
      statusEl.textContent = 'Update complete. Ready to play.';
    } else {
      statusEl.textContent = 'Up to date. Ready to play.';
    }
  } catch (e) {
    statusEl.textContent = 'Offline mode: ' + e.message;
  }
  newsEl.textContent = 'Welcome to ZONA. Press Play to start.';
}

playBtn.addEventListener('click', async () => {
  const channel = channelSel.value;
  statusEl.textContent = 'Checking updates...';
  try {
    const { manifest, needsUpdate } = await window.zona.checkUpdate(channel);
    versionEl.textContent = 'v' + manifest.version;
    if (needsUpdate) {
      statusEl.textContent = 'Downloading update...';
      await window.zona.downloadUpdate(manifest);
      statusEl.textContent = 'Update complete.';
    } else {
      statusEl.textContent = 'Up to date.';
    }
    statusEl.textContent = 'Launching game...';
    await window.zona.launchGame({ server: document.getElementById('server').value, port: 7777 });
    statusEl.textContent = 'Game launched.';
  } catch (e) {
    statusEl.textContent = 'Error: ' + e.message;
  }
});

init();
