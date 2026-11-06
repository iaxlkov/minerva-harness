# Minerva Harness

Harness с самостоятельным Rust runtime, web-интерфейсом React/assistant-ui и desktop-оболочкой Tauri. Один фронтенд работает через типизированный HTTP-клиент в браузере и через временный desktop-адаптер в Tauri.

## Возможности

- Провайдеры OpenAI Chat Completions, OpenAI Responses и Anthropic Messages, включая собственные совместимые серверы.
- Codex через установленный App Server: авторизация, каталог моделей, настройки и диалоги.
- SQLite: переключение сессий и восстановление текста, вызовов инструментов и их результатов; атомарная запись ответа и состояния интеграции.
- Встроенные сервисы Cordis: провайдеры, Codex и командная оболочка. UI собирается через типизированные plugin slots; локальные доверенные Rust-пакеты можно устанавливать как отдельные процессы. Внешний browser-loader, plugin ToolSet/approvals и полный hosted deployment ещё в работе.

## Запуск в браузере

Нужны Node.js 24 и Rust. На Windows нужен MSVC linker; WebView2 для web-режима не требуется.

```powershell
npm.cmd ci
npm.cmd run build
npm.cmd run server
```

Откройте одноразовую ссылку, которую сервер напечатает в терминал. Она действует пять минут; token обменивается на HttpOnly cookie и сразу удаляется из адреса страницы. Сервер по умолчанию слушает `127.0.0.1:3080`, данные находятся в `.minerva/data`, настройки — в `.minerva/config`. Другие пути задаются `--data-dir` и `--config-dir`. `0.0.0.0` в локальном режиме запрещён.

Для разработки запустите в двух терминалах `npm run dev:web` и `npm run dev`. Первый поднимает Rust API и печатает ссылку на frontend, второй — Vite на `127.0.0.1:1420` с proxy HTTP/WebSocket.

Принятый запуск принадлежит runtime: закрытие вкладки его не отменяет. Повтор с тем же requestId возвращает тот же run; при восстановлении соединения UI получает согласованный baseline и сохранённые события. Отмена — отдельная команда. Полного token streaming пока нет: UI отображает доступные структурированные checkpoints.

## Устанавливаемый Rust-плагин

```powershell
npm.cmd run plugin:example
npm.cmd run server -- --install-plugin agent-artifacts/review-example-package
```

Затем включите `Review example` на странице «Плагины». Пакет предоставляет `review.summary` через типизированный endpoint `plugins.call`; пример считает строки переданных текстов и не читает файлы. Установщик принимает готовый каталог с `plugin.json`, фиксирует digest и не выполняет build scripts. Native-пакеты являются доверенным локальным кодом; hosted-запуск таких пакетов закрыт до подключения sandbox provider. Сторонняя UI-часть этого примера пока не загружается.

## Структура

- `apps/web` — frontend, страницы и UI-композиция.
- `crates/runtime` — история, исполнение, провайдеры и Cordis.
- `crates/api`, `apps/server` — самостоятельный локальный HTTP/WS Host.
- `packages/client`, `packages/client-models`, `packages/plugin-sdk` — wire-клиент, наблюдаемые модели и UI SDK.
- `src-tauri` — desktop carrier с временными IPC-командами.
- `apps/gateway`, `apps/worker`, `apps/runner`, `crates/server-control` — реализуемая hosted-платформа; её итоговая приёмка ещё не пройдена.

Состояние этапов и ограничения: [фактический прогресс](docs/validation/web-platform-progress.md). Архитектура и отдельные планы: [web platform design](docs/superpowers/specs/2026-11-07-web-platform-design.md).

## Разработка

Нужны Node.js 24 с npm, Rust stable (MSVC), Visual Studio Build Tools с C++ и Windows SDK, WebView2 Runtime. Для Codex нужен отдельно установленный `codex` в PATH.

```powershell
npm.cmd ci
.\start-dev.cmd
```

На macOS нужны Node.js 24, Rust stable и Xcode Command Line Tools. Запуск:

```sh
npm ci
npm run tauri:dev
```

Интеграция Codex ищет `codex.exe` в Windows и `codex` в macOS/Linux через PATH. На macOS также поддерживается CLI внутри ChatGPT.app (в `/Applications` или `~/Applications`) и стандартные пути Homebrew. Вход в ChatGPT открывается в системном браузере; существующая локальная авторизация Codex используется автоматически.

Tauri собирает Rust, Vite обслуживает интерфейс с HMR на `127.0.0.1:1420`. Порт должен быть свободен; старую экспериментальную копию одновременно не запускайте.

## Проверки

```powershell
npm.cmd test
npm.cmd run build
npm.cmd run contracts:check
npm.cmd run test:e2e:web
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Rust требует доступного MSVC linker (например, Developer PowerShell for Visual Studio). Обычные тесты используют локальные моки; обращения к реальному Codex помечены `ignored`. Отдельный диагностический тест `migrate_backup_copy` использует `SESSION_V1_COPY` и рассчитан на копию прежней базы с двумя диалогами и четырьмя сообщениями, не на рабочую базу.

## Данные и ограничения

Идентификатор приложения сохранён как `ru.oestudio.rig.prototype` для совместимости с данными прототипа. В Windows desktop-история расположена в `%APPDATA%\ru.oestudio.rig.prototype\sessions.sqlite3`. При открытии v1–v4 создаётся согласованная SQLite backup-копия `*.pre-v5-*.sqlite3`, затем выполняется миграция на v5 с журналом событий. Если backup не создан, обновление прекращается. Старые бинарники не открывают новую схему; для отката нужна резервная копия. Рабочая БД не используется тестами.

API-ключи хранятся в Windows Credential Manager. Codex использует собственную локальную авторизацию. Данные, ключи, сборки и диагностические файлы не входят в репозиторий.

Вызовы, аргументы, результаты и ошибки инструментов сохраняются внутри ответа и восстанавливаются в чате. Промежуточные снимки сохраняются до завершения хода; после сбоя они помечаются как прерванные. Открытие истории не запускает инструменты. Полный поток событий, подключение оболочки к Rig и работа с вложениями ещё не завершены. Контракт и ограничения: [docs/structured-history.md](docs/structured-history.md). Это исходники разработки, не установочный релиз.

План аудита и следующие этапы: [docs/core-refactoring-plan.md](docs/core-refactoring-plan.md).
