#!/usr/bin/env node
// Одна версия на весь продукт: источник истины — [workspace.package] version
// в корневом Cargo.toml. Скрипт падает, если лаунчер или офлайн-манифест
// разошлись с ней, если в манифесте абсолютный путь, или если тег релиза
// не совпадает с версией в файлах.
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');

const root = path.resolve(__dirname, '..');
const problems = [];
const notes = [];

const fail = (msg) => problems.push(msg);

function readJson(rel) {
  try {
    return JSON.parse(fs.readFileSync(path.join(root, rel), 'utf8'));
  } catch (e) {
    fail(`${rel}: не читается (${e.message})`);
    return null;
  }
}

function workspaceVersion() {
  let text;
  try {
    text = fs.readFileSync(path.join(root, 'Cargo.toml'), 'utf8');
  } catch (e) {
    fail(`Cargo.toml: не читается (${e.message})`);
    return null;
  }
  const section = text.split(/^\[/m).find((s) => s.startsWith('workspace.package]'));
  if (!section) {
    fail('Cargo.toml: нет секции [workspace.package]');
    return null;
  }
  const m = section.match(/^version\s*=\s*"([^"]+)"/m);
  if (!m) {
    fail('Cargo.toml: нет version в [workspace.package]');
    return null;
  }
  return m[1];
}

const version = workspaceVersion();
if (version && !/^\d+\.\d+\.\d+$/.test(version)) {
  fail(`Cargo.toml: версия "${version}" не в формате X.Y.Z`);
}

if (version) {
  const pkg = readJson('launcher/package.json');
  if (pkg && pkg.version !== version) {
    fail(`launcher/package.json: ${pkg.version} != ${version} (Cargo.toml)`);
  }

  const stub = readJson('launcher/manifest.stub.json');
  if (stub) {
    if (stub.version !== version) {
      fail(`launcher/manifest.stub.json: ${stub.version} != ${version} (Cargo.toml)`);
    }
    const files = Array.isArray(stub.files) ? stub.files : [];
    if (files.length !== 1) {
      fail(`launcher/manifest.stub.json: ожидается один файл, найдено ${files.length}`);
    }
    for (const f of files) {
      if (typeof f.url !== 'string' || !f.url.startsWith('local:')) {
        fail(
          `launcher/manifest.stub.json: url "${f.url}" должен быть local:<путь от launcher/>` +
            ' — абсолютный file:/// путь не переживёт перенос репозитория',
        );
        continue;
      }
      const payload = path.join(root, 'launcher', f.url.slice('local:'.length));
      if (!fs.existsSync(payload)) {
        notes.push(`payload ${f.url} отсутствует — sha256 не проверен`);
        continue;
      }
      const buf = fs.readFileSync(payload);
      const sha = crypto.createHash('sha256').update(buf).digest('hex');
      if (sha !== f.sha256) {
        fail(
          `launcher/manifest.stub.json: sha256 не совпадает с ${f.url}` +
            ' — пересоберите: cd launcher && npm run stub',
        );
      }
      if (buf.length !== f.size) {
        fail(`launcher/manifest.stub.json: size ${f.size} != ${buf.length} для ${f.url}`);
      }
    }
  }
}

// В CI на теге проверяем, что версия в файлах не отстала от тега.
const tag = process.env.GITHUB_REF_NAME;
if (version && tag && /^v\d/.test(tag) && tag !== `v${version}`) {
  fail(`тег ${tag} не совпадает с версией v${version} (Cargo.toml) — поднимите версию перед тегом`);
}

for (const n of notes) console.log(`[versions] ${n}`);
if (problems.length) {
  for (const p of problems) console.error(`[versions] ${p}`);
  console.error(`[versions] ошибок: ${problems.length}`);
  process.exit(1);
}
console.log(`[versions] ok — ${version}`);
