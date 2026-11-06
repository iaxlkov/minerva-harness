# D. Desktop, развёртывание и выпуск — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Делегирование применяется только при отдельном разрешении пользователя.

**Goal:** Упаковать общую web/plugin платформу для Windows/macOS Tauri и Linux-сервера, проверить миграцию и восстановление.

**Architecture:** Tauri управляет локальным Host и отображает общий web-клиент. Hosted deployment использует тот же Host за gateway, OIDC и sandbox runner. Версии UI/runtime/SDK выпускаются согласованно.

**Tech Stack:** Tauri v2, текущий Rust/npm toolchain, Linux OCI/runsc, PostgreSQL, HTTPS reverse proxy, OIDC provider, browser E2E.

**Spec:** [Архитектура платформы](../specs/2026-11-07-web-platform-design.md).

## Global Constraints

- Серверный выпуск рассчитан на Linux x86_64.
- Desktop сохраняет Windows и macOS.
- Рабочая БД не используется для тестов.
- Вход пользователей — через OIDC, без собственной системы паролей.
- Наличие двух окон или вкладок не создаёт двух писателей одной workspace БД.
- Live deploy, публикация релиза и изменение пользовательских данных выполняются отдельным явно разрешённым действием; этот план готовит проверяемые artifacts.

## D1. Тонкая Tauri-оболочка

**Files:** перенести `src-tauri/` в `apps/desktop/src-tauri/`; создать `apps/desktop/src-tauri/src/{host_process,bootstrap}.rs`; изменить Tauri config/capabilities, npm scripts, `start-dev.cmd`; удалить временный `packages/client/src/tauri.ts` после перевода потребителей.

**Consumes:** private local bootstrap A, runtime/profile C, единый web dist.

**Produces:** launch/ready/shutdown протокол дочернего Host. Startup выдаёт локальный origin и одноразовый credential по приватному каналу, окно открывается после readiness. Бизнес-вызовов через invoke больше нет.

```json
{"kind":"ready","protocolVersion":1,"origin":"http://127.0.0.1:49152"}
```

Credential не входит в этот логируемый readiness frame; передаётся отдельно и не попадает в argv/logs. Host слушает назначенный ОС порт. Занятый профиль подключается к проверенному работающему Host либо выдаёт понятную ошибку, но не пытается открыть БД второй раз.

- [ ] Написать process tests: early exit, readiness timeout, protocol mismatch, второй запуск и shutdown без orphan process.
- [ ] Использовать packaged Host artifact с фиксированной версией. Сохранить identifier, app data paths, tray и поведение окна; child process на Windows стартует скрыто.
- [ ] Перевести Tauri на общий HTTP/WS клиент и web dist. Для локального UI/плагинов выбрать same-origin от локального Host, чтобы не добавлять широкие cross-origin permissions.
- [ ] Выделить local/remote window security: удалённый web origin не получает Tauri filesystem/shell/invoke capabilities. Login hosted в desktop использует поддерживаемый browser OIDC flow и проверенный callback, без хранения access token в localStorage.
- [ ] При remote connect не создавать local runtime. Disconnect/закрытие окна не останавливает удалённый run. При завершении локального приложения использовать явный graceful shutdown и recorded interruption после timeout.
- [ ] Собрать и проверить Windows/macOS приложения: история, cancel, tray, plugin UI и external Rust plugin; коммит `feat: use shared web runtime in desktop shell`.

## D2. Deployment artifacts для нескольких пользователей

**Files:** `deploy/{Dockerfile.gateway,Dockerfile.worker,compose.yml,Caddyfile,server.env.example}`, `deploy/runner/`, `deploy/README.md`, `.github/workflows/web-platform.yml`.

**Produces:** серверный комплект с HTTPS gateway, PostgreSQL, настроенным внешним OIDC, credential service и runner. Image digest и dependency locks фиксированы; тестовый OIDC issuer из B не используется production-конфигурацией.

```text
Internet → TLS reverse proxy → Gateway
Gateway → PostgreSQL / Credential service / private Workspace Host
Trusted Runner → runsc sandbox для Host и execution
Workspace/Plugin → только выданные volumes и разрешённый egress
```

- [ ] Создать multi-stage images без Tauri/system WebView зависимостей. Готовый web dist входит в gateway; worker image содержит runtime и builtin plugins.
- [ ] Разделить публичную, control и workload сети. Сокет orchestration доступен только runner; gateway/worker/plugin не получают Docker socket или privileged mode.
- [ ] Настроить HTTPS, websocket upgrade, bounded request size, readiness/liveness и shutdown. UI и API публикуются с одного origin; static index не кэшируется как immutable, assets имеют digest.
- [ ] В `server.env.example` перечислить issuer/client ID/redirect URL/public origin и ссылки на server secret files без значений. Production старт отказывает при test issuer, отсутствующем ключе, недоступном sandbox или внешнем local-auth bind.
- [ ] Добавить resource quotas и ограничение количества idle/active Host. Idle eviction допускается только без active runs; последующий cold start восстанавливает тот же workspace.
- [ ] Проверить clean Linux install по deploy/README двумя пользователями, active WS и external plugin. Зафиксировать фактические RAM/CPU/latency cold start; коммит `build: package isolated hosted deployment`.

## D3. Миграция данных и восстановление

**Files:** `crates/session/src/import.rs`, `apps/server/src/admin_import.rs`, `tests/migration/legacy-v4.test.mjs`, `scripts/backup-workspace.mjs`, `scripts/restore-workspace.mjs`, `docs/operations/{migration,backup-restore}.md`.

**Produces:** explicit import в назначенный workspace, backup manifest с версиями схемы/API/plugin lock и проверяемым digest. Команды выполняются с workspace ID и авторизованным владельцем; абсолютный путь от браузера не используется для выбора host-файла.

- [ ] Подготовить fixtures v1–v4 на копиях: текст, tool calls/results, interrupted run и integration state; сохранить ожидаемые ID/count/hash до импорта.
- [ ] Импортировать локальные данные в новый workspace через maintenance-operation, не смешивая чужие tenant. Ключи перенести отдельной процедурой владельца, не из произвольного общего keyring.
- [ ] Для SQLite использовать согласованный backup API или остановку писателя; не копировать только основной файл поверх активного WAL. Control DB и workspace snapshot связываются manifest ревизией.
- [ ] Проверить повторный import не дублирует данные, unsupported future schema отклоняется, ошибка транзакции сохраняет исходную копию.
- [ ] На чистом deployment восстановить control records, workspace history, plugins/config и отдельно keys. Проверить UI, run state и отсутствие tool re-execution при чтении.
- [ ] Документировать rollback: старая версия запускается только на совместимой схеме или восстановленной копии; неизменяемые artifacts сохраняются. Коммит `feat: migrate and restore workspace data safely`.

## D4. Общая матрица приёмки

**Files:** `tests/release/platform.spec.ts`, `docs/validation/web-platform-release.md`, `README.md`, `docs/core-refactoring-plan.md`.

| Проверка | Local web | Windows Tauri | macOS Tauri | Linux hosted |
|---|---|---|---|---|
| История/chat/cancel/reconnect | обязательно | обязательно | обязательно | обязательно |
| UI slots и Rust+UI plugin | обязательно | обязательно | обязательно | обязательно |
| Разделение двух пользователей | неприменимо: один local principal | неприменимо для local | неприменимо для local | обязательно |
| Remote hosted connection | browser | обязательно | обязательно | browser |
| Credentials и Codex | по объявленным capabilities | по capabilities | по capabilities | по подтверждённому hosted flow |
| Backup/restore и старые данные | обязательно | shared format | shared format | обязательно |

- [ ] Выполнить `npm test`, `npm run build`, `npm run contracts:check`, все web/hosted/plugin E2E scripts из A–C.
- [ ] Выполнить `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`; platform-specific targets проверить на соответствующей ОС.
- [ ] Проверить сборки Tauri на Windows и macOS и чистое Linux развёртывание. Непроверенная ОС явно отмечается как незавершённая приёмка, а не как passed.
- [ ] Повторить все cross-tenant отрицательные сценарии после включения внешнего плагина. Проверить missing/disabled renderer на истории, logout, stale session cookie и истёкший worker scope.
- [ ] Обновить README с фактическими командами local/web/desktop/server, plugin author guide и ограничениями. Старый core-refactoring-plan связать с новым решением, сохранив исторические результаты проверок.
- [ ] Записать точные artifact digests, версии, результаты, известные ограничения и способы восстановления; коммит `docs: record platform release validation`.

## Gate D и критерий выпуска

Все требования раздела «Критерии завершения всей программы» спецификации имеют фактическую проверку. Подготовлены artifacts и инструкция запуска; публикация/production deploy выполняются по отдельной команде пользователя. Отсутствие sandbox-тестов, проверок двух пользователей или external plugin E2E блокирует объявление многопользовательской версии готовой.
