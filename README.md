# Minerva Harness

Harness с самостоятельным Rust runtime, web-интерфейсом React/assistant-ui и desktop-оболочкой Tauri. Один фронтенд работает через типизированный HTTP-клиент в браузере и через временный desktop-адаптер в Tauri.

## Возможности

- Провайдеры OpenAI Chat Completions, OpenAI Responses и Anthropic Messages, включая собственные совместимые серверы.
- SQLite: переключение сессий и восстановление текста, вызовов инструментов и их результатов; атомарная запись ответа и состояния интеграции.
- Встроенные сервисы Cordis: провайдеры и командная оболочка. UI собирается через типизированные plugin slots; установленные пакеты подключают внешние React-модули с общими компонентами приложения. Локальные Rust-backend работают отдельными процессами. Plugin ToolSet/approvals и полный hosted deployment ещё в работе.

Codex подключается отдельным process-плагином: его исходники и executable не входят в workspace и базовую сборку. Пакет разрабатывается в соседнем `../minerva-codex-plugin` и использует общий Rust SDK. Прежние Codex-сессии не мигрируются.

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

### Codex в desktop Tauri

Пакет пока поддерживает Windows; на других ОС запуск явно отклоняется до реализации надёжного завершения дерева процессов. Нужен установленный Codex CLI в `PATH`. Соберите внешний пакет, затем установите его в каталог настроек desktop:

```powershell
powershell -File ../minerva-codex-plugin/package.ps1
npm.cmd run server -- --install-plugin ../minerva-codex-plugin/dist/codex-package --config-dir "$env:APPDATA/ru.oestudio.rig.prototype"
```

После сообщения об установке остановите временный сервер и запустите Tauri. На странице «Плагины» включите Codex. В общем окне «Настройки» появится пункт Codex: вход через CLI, аккаунт/лимиты, видимость и модель по умолчанию, параметры ответа. Модели появляются в обычном селекторе чата. История и продолжение thread сохраняются через общий engine-протокол; отмена завершает дерево процессов app-server. Плагин использует read-only sandbox без запросов approval.

UI установленного пакета — доверенный React-модуль SDK v2, загружаемый один раз на время включения плагина. Он использует общий React, UI-компоненты и слоты приложения; контроллеры сохраняют состояние между открытиями настроек. Это код в контексте приложения, а не sandbox. Настройки лежат в `plugins/.data/<id>`, отдельно от проверяемых digest файлов пакета. Без внешнего пакета приложение собирается и работает самостоятельно. Контракт и жизненный цикл: [UI плагинов](docs/plugin-ui.md).

Менеджер умеет приостанавливать все несистемные плагины, сохраняя индивидуальный выбор. Внешние backend могут объявлять именованные сервисы и зависимости; Host проверяет граф и управляет порядком запуска и остановки. Подробнее: [backend-плагины](docs/backend-plugins.md).

## Структура

- `apps/web` — frontend, страницы и UI-композиция.
- `crates/runtime` — история, исполнение, провайдеры и Cordis.
- `crates/api`, `apps/server` — самостоятельный локальный HTTP/WS Host.
- `packages/client`, `packages/client-models`, `packages/plugin-sdk` — wire-клиент, наблюдаемые модели и UI SDK.
- `src-tauri` — desktop carrier с временными IPC-командами.
- `apps/gateway`, `apps/worker`, `apps/runner`, `crates/server-control` — реализуемая hosted-платформа; её итоговая приёмка ещё не пройдена.

Состояние этапов и ограничения: [фактический прогресс](docs/validation/web-platform-progress.md). Архитектура и отдельные планы: [web platform design](docs/superpowers/specs/2026-11-07-web-platform-design.md).

## Разработка

Нужны Node.js 24 с npm, Rust stable (MSVC), Visual Studio Build Tools с C++ и Windows SDK, WebView2 Runtime.

```powershell
npm.cmd ci
.\start-dev.cmd
```

На macOS нужны Node.js 24, Rust stable и Xcode Command Line Tools. Запуск:

```sh
npm ci
npm run tauri:dev
```

На Linux (Ubuntu 24.04 / Debian) нужны Node.js 24, Rust stable и системные библиотеки WebKitGTK. Запуск тот же, что на macOS:

```sh
sudo apt install -y build-essential libssl-dev libwebkit2gtk-4.1-dev librsvg2-dev libayatana-appindicator3-dev pkg-config
npm ci
npm run tauri:dev
```

В виртуальной машине без 3D-ускорения окно может остаться пустым; тогда запускайте с `WEBKIT_DISABLE_DMABUF_RENDERER=1 npm run tauri:dev`.


Tauri собирает Rust, Vite обслуживает интерфейс с HMR на `127.0.0.1:1420`. Порт должен быть свободен; старую экспериментальную копию одновременно не запускайте.

## Проверки

Для AI-разработки подключён [Ponytail](docs/ponytail.md): общие правила в `AGENTS.md` и шесть project-local skills в `.agents/skills`.

```powershell
npm.cmd test
npm.cmd run build
npm.cmd run contracts:check
npm.cmd run test:e2e:web
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Rust требует доступного MSVC linker (например, Developer PowerShell for Visual Studio). Обычные тесты используют локальные моки. Отдельный диагностический тест `migrate_backup_copy` использует `SESSION_V1_COPY` и рассчитан на копию прежней базы с двумя диалогами и четырьмя сообщениями, не на рабочую базу.

## Данные и ограничения

Идентификатор приложения сохранён как `ru.oestudio.rig.prototype` для совместимости с данными прототипа. В Windows desktop-история расположена в `%APPDATA%\ru.oestudio.rig.prototype\sessions.sqlite3`. Текущая схема — v6 с журналом событий и внутренними сессиями экспериментов. Перед миграцией v1–v4 создаётся SQLite backup `*.pre-v5-*.sqlite3`, перед v5→v6 — `*.pre-v6-*.sqlite3`. Если backup не создан, обновление прекращается. Старые бинарники не открывают новую схему; для отката нужна резервная копия. Рабочая БД не используется тестами.

API-ключи хранятся в Windows Credential Manager. Данные, ключи, сборки и диагностические файлы не входят в репозиторий.

Композиция плагинов и параметры встроенных сервисов задаются в `minerva.conf.json`.
Desktop создаёт файл в `%APPDATA%\ru.oestudio.rig.prototype`; существующий выбор
плагинов и лимиты shell импортируются автоматически. [Формат и правила сохранения](docs/configuration.md).

Вызовы, аргументы, результаты и ошибки инструментов сохраняются внутри ответа и восстанавливаются в чате. Промежуточные снимки сохраняются до завершения хода; после сбоя они помечаются как прерванные. Открытие истории не запускает инструменты. Полный поток событий, подключение оболочки к Rig и работа с вложениями ещё не завершены. Контракт и ограничения: [docs/structured-history.md](docs/structured-history.md). Это исходники разработки, не установочный релиз.

План аудита и следующие этапы: [docs/core-refactoring-plan.md](docs/core-refactoring-plan.md).

Harness execution, tools, approvals, MCP and context: [usage and limits](docs/harness.md).
Память, поиск истории, версии навыков и Propose–Probe–Compose: [локальный контур обучения](docs/harness-learning.md).
External Codex source: [kviron/minerva-codex-plugin](https://github.com/kviron/minerva-codex-plugin) (separate private repository).
