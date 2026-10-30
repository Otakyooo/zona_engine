const channelSel = document.getElementById('channel');
const statusEl = document.getElementById('status');
const newsEl = document.getElementById('news');
const playBtn = document.getElementById('play');
const progressBar = document.getElementById('progress-bar');
const progressText = document.getElementById('progress-text');
const versionEl = document.getElementById('version');
const localVersionEl = document.getElementById('local-version');

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
  newsEl.textContent = 'Welcome to ZONA. Check for updates before playing.';
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
    await window.zona.launchGame({});
    statusEl.textContent = 'Game launched.';
  } catch (e) {
    statusEl.textContent = 'Error: ' + e.message;
  }
});

init();
