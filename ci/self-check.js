#!/usr/bin/env node
// Петля уробороса: код -> тесты/метрики -> память/журнал -> следующий шаг.
//
//   node ci/self-check.js                 проверки + генерация артефактов
//   node ci/self-check.js --verify       только проверки
//   node ci/self-check.js --generate     только генерация
//   node ci/self-check.js --with-cargo   сосчитать тесты локально через cargo
//   ZONA_TEST_COUNT=144 node ci/self-check.js --verify --generate  (режим CI)
//
// Внешних npm-зависимостей нет: ci/roadmap.yaml — это JSON (подмножество YAML 1.2).
// Генерация детерминированная: в отслеживаемые файлы не попадают HEAD, дата и
// коммиты с [skip ci]; файл перезаписывается только при реальном diff, поэтому
// автокоммит ROADMAP.md в CI не порождает бесконечный цикл.
const fs = require('fs');
const path = require('path');
const { spawnSync, execFileSync } = require('child_process');

const root = path.resolve(__dirname, '..');
const args = process.argv.slice(2);
const wantVerify = args.includes('--verify');
const wantGenerate = args.includes('--generate');
const explicitMode = wantVerify || wantGenerate;
const withCargo = args.includes('--with-cargo');
const doVerify = explicitMode ? wantVerify : true;
const doGenerate = explicitMode ? wantGenerate : true;
const problems = [];
const notes = [];
const fail = (msg) => problems.push(msg);

function read(rel) {
  try {
    return fs.readFileSync(path.join(root, rel), 'utf8');
  } catch (e) {
    return null;
  }
}

function readJson(rel) {
  const text = read(rel);
  if (text === null) return null;
  try {
    return JSON.parse(text);
  } catch (e) {
    fail(`${rel}: не читается как JSON (${e.message})`);
    return null;
  }
}

function git(fileArgs, fallback = '') {
  try {
    return execFileSync('git', fileArgs, {
      cwd: root,
      encoding: 'utf8',
      stdio: ['ignore', 'pipe', 'ignore'],
    }).trim();
  } catch (e) {
    notes.push(`git ${fileArgs[0]} недоступен — использована заглушка`);
    return fallback;
  }
}

function writeIfChanged(rel, content) {
  const full = path.join(root, rel);
  fs.mkdirSync(path.dirname(full), { recursive: true });
  const before = fs.existsSync(full) ? fs.readFileSync(full, 'utf8') : null;
  if (before === content) return 'unchanged';
  fs.writeFileSync(full, content, 'utf8');
  return before === null ? 'created' : 'updated';
}

// ---------------------------------------------------------------- данные

function loadRoadmap() {
  const raw = read('ci/roadmap.yaml');
  if (raw === null) {
    fail('ci/roadmap.yaml отсутствует — это единственный источник ROADMAP.md');
    return null;
  }
  // Комментарии допустимы только отдельными строками до начала объекта.
  const json = raw.split(/\r?\n/).filter((l) => !/^\s*#/.test(l)).join('\n');
  try {
    const data = JSON.parse(json);
    if (!Array.isArray(data.phases) || data.phases.length === 0) {
      fail('ci/roadmap.yaml: нет непустого массива phases');
      return null;
    }
    for (const p of data.phases) {
      for (const key of ['id', 'version', 'priority', 'status', 'title', 'goal', 'tasks', 'criteria']) {
        if (!(key in p)) fail(`ci/roadmap.yaml: у фазы ${p.id || '?'} нет поля ${key}`);
      }
    }
    return data;
  } catch (e) {
    fail(`ci/roadmap.yaml: не парсится (${e.message})`);
    return null;
  }
}

function currentFacts() {
  const cargo = read('Cargo.toml') || '';
  const net = read('crates/engine/zn-net/src/lib.rs') || '';
  const version = (cargo.match(/^version\s*=\s*"([^"]+)"/m) || [])[1] || '?';
  const protocol = Number((net.match(/PROTOCOL_VERSION\s*:\s*u16\s*=\s*(\d+)/) || [])[1] || 0);
  const frameMatch = net.match(/MAX_FRAME_BYTES\s*:\s*usize\s*=\s*([^;]+);/);
  return {
    version,
    protocol,
    frameExpr: frameMatch ? frameMatch[1].trim() : '?',
    // HEAD намеренно не читается: он менялся бы в каждом коммите и делал бы
    // отслеживаемые документы недетерминированными.
    lastTag: git(['describe', '--tags', '--abbrev=0'], ''),
  };
}

function testCount() {
  const stored = readJson('ci/artifacts/tests.json');
  const previous = stored && typeof stored.count === 'number' ? stored.count : null;
  let count = previous;
  let source = previous !== null ? 'ci/artifacts/tests.json' : 'baseline из ci/roadmap.yaml';
  let measured = false;

  if (withCargo) {
    const res = spawnSync('cargo', ['test', '--workspace', '--', '--list'], {
      cwd: root,
      encoding: 'utf8',
      env: { ...process.env, CARGO_INCREMENTAL: '0' },
      maxBuffer: 32 * 1024 * 1024,
    });
    if (res.status !== 0) {
      fail('cargo test --workspace -- --list завершился с ошибкой');
    } else {
      const n = (res.stdout || '').split(/\r?\n/).filter((l) => /:\s*test\s*$/.test(l)).length;
      if (n > 0) {
        count = n;
        source = 'cargo test --workspace -- --list';
        measured = true;
      } else {
        fail('не удалось сосчитать тесты в выводе cargo test --list');
      }
    }
  } else if (process.env.ZONA_TEST_COUNT !== undefined) {
    // Число посчитано в build-rust (там уже есть toolchain и тёплый target/).
    const n = Number(process.env.ZONA_TEST_COUNT);
    if (Number.isInteger(n) && n > 0) {
      count = n;
      source = 'build-rust (cargo test --workspace -- --list)';
      measured = true;
    } else {
      fail(`ZONA_TEST_COUNT="${process.env.ZONA_TEST_COUNT}" — ожидается положительное целое`);
    }
  }

  if (measured && previous !== null && count < previous) {
    fail(`регрессия тестов: было ${previous}, стало ${count}`);
  }
  return { count, source, previous, measured };
}

// ---------------------------------------------------------------- проверки

function verify(facts, roadmap, tests) {
  const res = spawnSync(process.execPath, [path.join(root, 'ci', 'check-versions.js')], {
    cwd: root,
    encoding: 'utf8',
    env: process.env,
  });
  if (res.stdout) process.stdout.write(res.stdout);
  if (res.stderr) process.stderr.write(res.stderr);
  if (res.status !== 0) fail('ci/check-versions.js не прошёл');

  if (facts.protocol <= 0) fail('не найден PROTOCOL_VERSION в crates/engine/zn-net/src/lib.rs');
  if (facts.frameExpr === '?') fail('не найден MAX_FRAME_BYTES в crates/engine/zn-net/src/lib.rs');
  else if (!/64\s*\*\s*1024/.test(facts.frameExpr)) {
    notes.push(`MAX_FRAME_BYTES = ${facts.frameExpr} — отличается от 64 КиБ, проверить лимит кадра вручную`);
  }

  const stub = readJson('launcher/manifest.stub.json');
  if (stub && typeof stub.protocol === 'number' && stub.protocol !== facts.protocol) {
    fail(`manifest.stub.json: protocol ${stub.protocol} != PROTOCOL_VERSION ${facts.protocol}`);
  } else if (stub && typeof stub.protocol !== 'number') {
    notes.push('manifest.stub.json: поле protocol появится в Фазе 2 вместе с bump протокола');
  }

  if (tests.count !== null && tests.count < roadmap.baseline.tests) {
    fail(`регрессия тестов: ${tests.count} < baseline ${roadmap.baseline.tests}`);
  }

  const badStatus = roadmap.phases.find((p) => !['planned', 'in_progress', 'done'].includes(p.status));
  if (badStatus) fail(`ci/roadmap.yaml: неизвестный статус "${badStatus.status}" у ${badStatus.id}`);
}

// ---------------------------------------------------------------- генерация

const escapeCell = (s) => String(s).replace(/\|/g, '\\|').replace(/\r?\n/g, ' ');
const bullet = (arr) => (arr || []).map((x) => `- ${x}`).join('\n');
const CHANGE_TYPES = [
  ['feat', 'Новое'],
  ['fix', 'Исправления'],
  ['docs', 'Документация и память'],
  ['chore', 'Релизы и инфраструктура'],
  ['refactor', 'Рефакторинг'],
  ['test', 'Тесты'],
];

function logLines(range) {
  const log = git(['log', '--pretty=format:%h%x09%s', range], '');
  // [skip ci] — автокоммиты генератора: их нельзя включать в changelog,
  // иначе документ породит сам себя.
  return log ? log.split(/\r?\n/).filter(Boolean).filter((l) => !/\[skip ci\]/i.test(l)) : [];
}

function groupCommits(lines) {
  const L = [];
  for (const [type, title] of CHANGE_TYPES) {
    const items = lines.filter((l) => new RegExp(`^[0-9a-f]+\\t${type}(\\([^)]*\\))?!?:`, 'i').test(l));
    if (items.length === 0) continue;
    L.push(`### ${title}`, '');
    for (const item of items) {
      const [hash, ...rest] = item.split('\t');
      L.push(`- \`${hash}\` ${rest.join('\t')}`);
    }
    L.push('');
  }
  const other = lines.filter((l) => !CHANGE_TYPES.some(([t]) => new RegExp(`^[0-9a-f]+\\t${t}(\\([^)]*\\))?!?:`, 'i').test(l)));
  if (other.length) {
    L.push('### Прочее', '');
    for (const item of other) {
      const [hash, ...rest] = item.split('\t');
      L.push(`- \`${hash}\` ${rest.join('\t')}`);
    }
    L.push('');
  }
  return L;
}

function renderRoadmap(data, facts, tests) {
  const labels = data.status_labels || {};
  // Источник числа тестов в отслеживаемый файл не пишем: он differs между
  // локальным --with-cargo и CI (ZONA_TEST_COUNT) и создавал бы осцилляцию diff.
  const testsText = tests.count === null
    ? `${data.baseline.tests} (baseline ${data.baseline.commit}, ещё не измерено cargo)`
    : `${tests.count}`;
  const L = [];
  L.push('# ZONA Engine — дорожная карта');
  L.push('');
  L.push('> **Сгенерировано автоматически. Не редактировать вручную.**');
  L.push('> Источник: `ci/roadmap.yaml`, генератор: `ci/self-check.js`. Статусы и метрики обновляются на каждом прогоне CI.');
  L.push('');
  L.push(`**Ось (уроборос):** ${data.axis}`);
  L.push('');
  L.push('| Показатель | Значение | Источник |');
  L.push('|---|---|---|');
  L.push(`| Версия | ${facts.version} | \`Cargo.toml\` [workspace.package] |`);
  L.push(`| Протокол | ${facts.protocol} | \`crates/engine/zn-net/src/lib.rs:17\` |`);
  L.push(`| Тесты | ${testsText} | \`cargo test --workspace -- --list\` |`);
  L.push(`| Baseline | ${data.baseline.tests} тестов, ${data.baseline.version}, протокол ${data.baseline.protocol}, ${data.baseline.commit} | \`ci/roadmap.yaml\` |`);
  L.push(`| Лимит кадра | \`${facts.frameExpr}\` | \`zn-net/src/lib.rs\` |`);
  L.push(`| Последний тег | ${facts.lastTag || '—'} | \`git describe\` |`);
  L.push('');
  L.push('## Фазы');
  L.push('');
  L.push('| Фаза | Версия | Приоритет | Статус | Цель | Критерий готовности | Следующий шаг |');
  L.push('|---|---|---|---|---|---|---|');
  for (const p of data.phases) {
    const next = p.status === 'done' ? '—' : (p.tasks[0] || '—');
    L.push(`| ${p.id} | ${escapeCell(p.version)} | ${p.priority} | ${labels[p.status] || p.status} | ${escapeCell(p.goal)} | ${escapeCell(p.criteria[0] || '—')} | ${escapeCell(next)} |`);
  }
  L.push('');
  for (const p of data.phases) {
    L.push(`### ${p.id} — ${p.version} «${p.title}» (${p.priority}, ${labels[p.status] || p.status})`);
    L.push('');
    L.push(p.goal);
    L.push('');
    L.push('**Задачи**');
    L.push('');
    L.push(bullet(p.tasks));
    L.push('');
    L.push('**Критерии готовности**');
    L.push('');
    L.push(bullet(p.criteria));
    L.push('');
    L.push(`- Источник метрик: ${p.metric_source || '—'}`);
    L.push(`- Зависимости: ${p.depends || '—'}`);
    L.push(`- Риски: ${p.risks || '—'}`);
    L.push('');
  }
  L.push('## Как замыкается петля');
  L.push('');
  L.push('`ci/self-check.js` читает `ci/roadmap.yaml`, `Cargo.toml`, `PROTOCOL_VERSION` и `MAX_FRAME_BYTES` из `crates/engine/zn-net/src/lib.rs`, число тестов из `ci/artifacts/tests.json` (или `ZONA_TEST_COUNT`/`--with-cargo`), а также `git log` и список тегов. Из этого он генерирует этот файл, `CHANGELOG.md` и неотслеживаемые черновики `ci/artifacts/next-step.md`, `journal-draft.md`, `CHANGELOG-unreleased.md`; при регрессии числа тестов и расхождении версий падает. В CI `ROADMAP.md` коммитится только при реальном diff и с `[skip ci]`, иначе возник бы бесконечный цикл. Чтение `logs/metrics.jsonl`, `ci/artifacts/load.json`, `OPEN.md` и `GUI_CHECKLIST.md` запланировано в Фазе 3. Живую GUI-проверку вносит только владелец.');
  L.push('');
  L.push('## Сквозные правила');
  L.push('');
  L.push(bullet(data.principles));
  L.push('');
  L.push('## Риски и обходы');
  L.push('');
  L.push('| Риск | Обход |');
  L.push('|---|---|');
  for (const r of data.risks || []) L.push(`| ${escapeCell(r.risk)} | ${escapeCell(r.mitigation)} |`);
  L.push('');
  L.push('## Что не делаем');
  L.push('');
  L.push(bullet(data.rejected));
  L.push('');
  L.push('---');
  L.push('');
  L.push('Сгенерировано из `ci/roadmap.yaml` · чтобы изменить статус или задачу, правьте источник, не этот файл.');
  L.push('');
  return L.join('\n');
}

// Отслеживаемый CHANGELOG — чистая функция тегов: он не меняется между
// релизами, поэтому CI не создаёт шумные автокоммиты.
function renderChangelog() {
  const tags = git(['tag', '--list', '--sort=version:refname'], '').split(/\r?\n/).filter(Boolean);
  const L = ['# CHANGELOG', '', '> **Сгенерировано автоматически** `ci/self-check.js` по тегам и Conventional Commits. Не редактировать вручную.', ''];
  for (let i = tags.length - 1; i >= 0; i -= 1) {
    const tag = tags[i];
    const range = i > 0 ? `${tags[i - 1]}..${tag}` : tag;
    const lines = logLines(range);
    L.push(`## ${tag}`, '');
    if (lines.length === 0) L.push('Нет коммитов, подходящих для changelog.', '');
    else L.push(...groupCommits(lines));
  }
  L.push('---', '', 'Коммиты после последнего тега см. в `ci/artifacts/CHANGELOG-unreleased.md` (не отслеживается git).', '');
  return L.join('\n');
}

function renderUnreleased(facts) {
  const range = facts.lastTag ? `${facts.lastTag}..HEAD` : 'HEAD';
  const lines = logLines(range);
  const L = ['# Не вошло в релиз', '', `Диапазон: \`${range}\`. Черновик, не отслеживается git.`, ''];
  if (lines.length === 0) L.push('Изменений нет.', '');
  else L.push(...groupCommits(lines));
  return L.join('\n');
}

function renderNextStep(data, facts, tests) {
  const active = data.phases.find((p) => p.status !== 'done');
  const L = ['# Следующий шаг', '', '> **Сгенерировано автоматически** `ci/self-check.js`. Черновик, не отслеживается git.', ''];
  if (!active) {
    L.push('Все фазы закрыты. Осталась приёмка владельцем и релиз.');
  } else {
    L.push(`**${active.id} — ${active.version} «${active.title}»** (${active.priority}, ${data.status_labels[active.status] || active.status})`);
    L.push('');
    L.push(`Цель: ${active.goal}`);
    L.push('');
    L.push(`Минимальное действие: ${active.tasks[0]}`);
    L.push('');
    L.push('Критерии готовности:');
    L.push('');
    L.push(bullet(active.criteria));
    L.push('');
    L.push(`Зависимости: ${active.depends} · Источник метрик: ${active.metric_source}`);
  }
  L.push('');
  L.push('## Ожидает владельца');
  L.push('');
  L.push('- `.workbuddy-ai/memory/GUI_CHECKLIST.md` — живая GUI-проверка (сетки, перенос/поворот, Settings, RTT, откат выстрела, смерть/эвакуация); GUI в среде агента не поднимается.');
  L.push('- Релизы и теги создаются только по явному запросу.');
  L.push('');
  L.push(`Снимок: версия ${facts.version}, протокол ${facts.protocol}, тесты ${tests.count === null ? data.baseline.tests + ' (baseline)' : tests.count}.`);
  L.push('');
  return L.join('\n');
}

function renderJournalDraft(data, facts, tests) {
  return [
    '# Заготовка записи журнала',
    '',
    '> Сгенерировано `ci/self-check.js`; перенести в `.workbuddy-ai/memory/journal/<дата>.md` с реальными числами и хешами. Дата и хеш проставляются при переносе, чтобы генерация оставалась детерминированной.',
    '',
    `- Версия: ${facts.version}, протокол: ${facts.protocol} (хеш коммита см. \`git log -1\`)`,
    `- Тесты: ${tests.count === null ? `${data.baseline.tests} (baseline)` : tests.count}`,
    `- Baseline: ${data.baseline.tests} тестов, ${data.baseline.version}, протокол ${data.baseline.protocol}, ${data.baseline.commit}`,
    '- Проверки: `cargo fmt --all -- --check`, `cargo build --workspace`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings` (всегда `CARGO_INCREMENTAL=0`)',
    '- Что изменено, почему, какие числа: _заполнить вручную_',
    '- Неочевидное решение → `DECISIONS.md`; ловушка → `PITFALLS.md` (симптом → причина → что делать)',
    '',
  ].join('\n');
}

// ---------------------------------------------------------------- запуск

const roadmap = loadRoadmap();
if (!roadmap) {
  for (const p of problems) console.error(`[self-check] ${p}`);
  process.exit(1);
}
const facts = currentFacts();
const tests = testCount();

if (doVerify) verify(facts, roadmap, tests);

if (doGenerate) {
  const outputs = [
    ['ROADMAP.md', renderRoadmap(roadmap, facts, tests)],
    ['CHANGELOG.md', renderChangelog()],
    ['ci/artifacts/CHANGELOG-unreleased.md', renderUnreleased(facts)],
    ['ci/artifacts/next-step.md', renderNextStep(roadmap, facts, tests)],
    ['ci/artifacts/journal-draft.md', renderJournalDraft(roadmap, facts, tests)],
  ];
  if (tests.measured && tests.count !== null) {
    outputs.push([
      'ci/artifacts/tests.json',
      JSON.stringify({ count: tests.count, version: facts.version, protocol: facts.protocol }, null, 2) + '\n',
    ]);
  }
  for (const [rel, content] of outputs) {
    const state = writeIfChanged(rel, content);
    notes.push(`${rel}: ${state}`);
  }
}

for (const n of notes) console.log(`[self-check] ${n}`);
if (problems.length) {
  for (const p of problems) console.error(`[self-check] ОШИБКА: ${p}`);
  console.error(`[self-check] ошибок: ${problems.length}`);
  process.exit(1);
}
console.log(`[self-check] ok — ${facts.version}, протокол ${facts.protocol}, тесты ${tests.count === null ? roadmap.baseline.tests + ' (baseline)' : tests.count}`);
