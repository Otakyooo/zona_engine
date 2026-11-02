# План: Самообновление лаунчера (electron-updater + NSIS)

## Цель

Сделать так, чтобы лаунчер обновлял сам себя через electron-updater с GitHub Releases, используя NSIS-установщик. Результат: при выходе новой версии лаунчер автоматически скачивает и устанавливает обновление.

## Контекст

Лаунчер обновляет только игру (через manifest.json), но не сам себя. Ранее electron-updater был удалён из-за краша (вызов до app.ready). Build target — portable, который не поддерживает автообновление. Нужно перейти на NSIS и корректно инициализировать autoUpdater.

## Зафиксированные решения

- Build target: NSIS (вместо portable) — поддерживает автообновление.
- Публикация: electron-builder `--publish always` с `GH_TOKEN` в CI (создаёт release + latest.yml).
- autoUpdater: инициализация внутри `app.whenReady()`, не на верхнем уровне.
- Проверка подписи: отключена для dev (`autoUpdater.verifyUpdateCodeSignature = false`).
- Канал: latest (stable).

## Затронутые границы

- `launcher/package.json`: build target nsis, publish config github, author.
- `launcher/main.js`: re-add autoUpdater, инициализация в whenReady, обработка событий.
- `ci/ci.yml` (и `.github/workflows/ci.yml`): release job публикует лаунчер через electron-builder с GH_TOKEN, затем загружает Rust-бинари и manifest.

## Порядок работ

### 1. package.json
- `"build": { "appId": "com.otakyooo.zona", "publish": { "provider": "github", "owner": "Otakyooo", "repo": "zona_engine" }, "win": { "target": "nsis" } }`
- Оставить electron в devDependencies.

### 2. main.js: autoUpdater
- `const { autoUpdater } = require('electron-updater');`
- В `app.whenReady()`: 
  - `autoUpdater.autoDownload = true;`
  - `autoUpdater.verifyUpdateCodeSignature = false;` (dev)
  - `autoUpdater.checkForUpdatesAndNotify();`
- Обработчики: `update-available` (лог), `update-downloaded` (предложить перезапуск), `error` (лог).
- `autoUpdater.on('update-downloaded', () => { autoUpdater.quitAndInstall(); })` (или с диалогом).

### 3. CI release job
- Добавить шаг публикации лаунчера:
  ```yaml
  - name: Publish launcher
    working-directory: launcher
    env:
      GH_TOKEN: ${{ secrets.GITHUB_TOKEN }}
    run: npm ci && npm run build -- --publish always
  ```
- Затем загрузить Rust-бинари и manifest в тот же release (softprops или gh release upload).
- Убедиться, что `latest.yml` загружен electron-builder.

### 4. Валидация
- Локально: `npm run build` (nsis) не падает.
- `npm start` (с unset env) запускается, autoUpdater не крашится.
- После публикации: лаунчер старой версии находит обновление и устанавливает.

## Риски

- electron-updater для NSIS требует, чтобы установщик был подписан или `verifyUpdateCodeSignature=false`. Для dev — ок, для продакшена нужен сертификат.
- Публикация через electron-builder и softprops в одном release: возможны конфликты. Рекомендуется использовать только electron-builder для лаунчера, а Rust-бинари загружать через `gh release upload` после создания release electron-builder.
- latest.yml должен быть в release; electron-builder создаёт его автоматически при publish.

## Вне области

- Подпись кода (EV/OV).
- Дифференциальные патчи.
- Тихая установка без UAC.

## Следующий шаг

Реализация агентом с правами на запись/сборку, начиная с package.json и main.js.
