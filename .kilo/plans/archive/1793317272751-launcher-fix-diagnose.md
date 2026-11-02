# План: Исправление лаунчера (выбор папки, запуск, диагностика)

## Цель

Устранить проблемы, из-за которых установленный лаунчер не позволяет выбрать папку установки и не запускает игру. Добавить диагностику и надёжную обработку ошибок.

## Контекст

Пользователь скачал `zona-launcher-Setup-0.1.0.exe` с GitHub Releases, установил, но:
- Не может выбрать папку установки (диалог не открывается или не работает).
- Не может запустить игру (ничего не происходит).

Возможные причины:
1. Переменная окружения `ELECTRON_RUN_AS_NODE` установлена глобально → Electron работает в режиме Node, `app`/`dialog` недоступны.
2. Манифест не скачивается (сеть/GitHub rate limit) → fallback на пустой stub → `needsUpdate=false` → игра не скачивается → `zn-client.exe` отсутствует → запуск молча падает.
3. `dialog.showOpenDialog` без окна может не работать в некоторых версиях Electron.
4. `launch-game` возвращает `{ok:true}` даже если exe не существует.

## Зафиксированные решения

- Добавить проверку `ELECTRON_RUN_AS_NODE` при старте и показать понятное сообщение (через консоль/диалог).
- Исправить `dialog.showOpenDialog` — передавать `BrowserWindow.getFocusedWindow()`.
- В `launch-game`: проверять существование `zn-client.exe`, возвращать `{ok:false, error}` если отсутствует.
- В `check-update`: если манифест пустой (нет files) — возвращать `needsUpdate: true` при отсутствии локальной игры.
- В renderer: показывать ошибки явно, не скрывать; добавить кнопку «Скачать/Обновить» отдельно от Play.
- Логировать ошибки в файл (userData/launcher.log).

## Затронутые границы

- `launcher/main.js`: `pick-install-dir`, `launch-game`, `fetchManifest`, логирование.
- `launcher/preload.js`: без изменений (API уже есть).
- `launcher/renderer.js`: обработка ошибок, отдельная кнопка Download.
- `launcher/index.html`: кнопка Download, область статуса.
- `README.md`: предупреждение про `ELECTRON_RUN_AS_NODE`.

## Порядок работ

### 1. Проверка ELECTRON_RUN_AS_NODE
- В `main.js` при старте: если `process.env.ELECTRON_RUN_AS_NODE` установлен — показать MessageBox с инструкцией (unset variable) и выйти.

### 2. Исправление диалога выбора папки
- `dialog.showOpenDialog(BrowserWindow.getFocusedWindow() || null, { properties: ['openDirectory'] })`.

### 3. Надёжный launch-game
- Проверить `fs.existsSync(exe)`. Если нет — вернуть `{ok:false, error:'Game not installed. Click Update first.'}`.
- В renderer: при ошибке показать сообщение и не скрывать.

### 4. Надёжный fetchManifest
- Если fetch с GitHub не удался — вернуть `{version:'0.0.0', files:[], error: ...}`.
- В `check-update`: `needsUpdate = manifest.files.length === 0 ? !fs.existsSync(exe) : manifest.version !== local.version`.

### 5. Отдельная кнопка «Скачать/Обновить»
- В UI добавить кнопку «Update», которая вызывает checkUpdate + downloadUpdate.
- Play проверяет наличие exe; если нет — предлагает обновить.

### 6. Логирование
- Записывать ошибки в `userData/launcher.log` (append).

### 7. README
- Добавить раздел «Устранение неполадок» с проверкой `ELECTRON_RUN_AS_NODE`.

## Риски

- GitHub rate limiting для манифеста — кэшировать последний успешный манифест в userData.
- Диалог выбора папки может не работать в упакованном приложении без окна — передавать окно.

## Следующий шаг

Реализация агентом с правами на запись/сборку.
