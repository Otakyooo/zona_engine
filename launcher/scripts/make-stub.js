#!/usr/bin/env node
// Пересобирает launcher/manifest.stub.json из офлайн-payload.
// Путь пишется относительным (local:game/zn-client.exe), поэтому манифест
// переживает перенос репозитория и машины. Версия берётся из package.json,
// а package.json сверяется с корневым Cargo.toml скриптом ci/check-versions.js.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const launcherDir = path.resolve(__dirname, '..');
const rel = (process.argv[2] || path.join('game', 'zn-client.exe')).split(path.sep).join('/');
const payload = path.join(launcherDir, rel);

if (!fs.existsSync(payload)) {
  console.error(`[stub] нет payload: ${payload}`);
  console.error('[stub] сначала соберите клиент: cargo build --release -p zn-client');
  process.exit(1);
}

const pkg = JSON.parse(fs.readFileSync(path.join(launcherDir, 'package.json'), 'utf8'));
const buf = fs.readFileSync(payload);
const sha = crypto.createHash('sha256').update(buf).digest('hex');
const manifest = {
  version: pkg.version,
  files: [{ path: path.basename(payload), url: `local:${rel}`, sha256: sha, size: buf.length }],
};

fs.writeFileSync(
  path.join(launcherDir, 'manifest.stub.json'),
  JSON.stringify(manifest, null, 2) + '\n',
);
console.log(
  `[stub] ${manifest.files[0].url} — ${buf.length} B, sha256 ${sha.slice(0, 12)}…, версия ${pkg.version}`,
);
