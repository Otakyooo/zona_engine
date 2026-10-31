# ZONA — Extraction Shooter

## Для игроков (тестеров)

1. Скачайте лаунчер с [GitHub Releases](https://github.com/Otakyooo/zona_engine/releases/latest).
2. Распакуйте архив в любую папку.
3. Запустите `zona-launcher.exe` (убедитесь, что переменная `ELECTRON_RUN_AS_NODE` не установлена).
4. Введите адрес сервера (по умолчанию `192.168.0.47`) и нажмите **Play**.
5. Лаунчер автоматически скачает игру и запустит её.

## Управление

- WASD — движение
- Мышь — камера
- ЛКМ — выстрел
- E — подбор
- ESC — освободить курсор

## Для разработчиков

### Сборка
```
cargo build --workspace
cargo test --workspace
cargo clippy --all-targets -- -D warnings
```

### Запуск локально
```
cargo run -p zn-server
cargo run -p zn-client
```

### Редактор
```
cargo run -p zn-tools --bin zn-editor
```

### Лаунчер
```
cd launcher
npm install
npm start
```

## Устранение неполадок

### Лаунчер не запускается / не работает (пустое окно, нет кнопок)
Переменная `ELECTRON_RUN_AS_NODE` может быть установлена в системе. Удалите её:
- PowerShell: `Remove-Item Env:ELECTRON_RUN_AS_NODE`
- CMD: `set ELECTRON_RUN_AS_NODE=`
- Проверка: `echo %ELECTRON_RUN_AS_NODE%` (должно быть пусто)

### Игра не скачивается
- Проверьте интернет-соединение.
- Нажмите «Download / Update» вручную.
- Проверьте лог: `%APPDATA%\zona-launcher\launcher.log`

### Не удаётся выбрать папку установки
- Убедитесь, что лаунчер запущен не в режиме Node (см. выше).
- Попробуйте запустить от имени администратора.
- `crates/` — Rust-движок и игра
- `launcher/` — Electron-лаунчер
- `deploy/` — скрипты деплоя сервера
- `ci/` — GitHub Actions
