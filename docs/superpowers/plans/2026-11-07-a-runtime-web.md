# A. Runtime и web-клиент — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Делегирование применяется только при отдельном разрешении пользователя.

**Goal:** Запустить существующий чат в обычном браузере через самостоятельный Rust Host с независимыми runs и восстановлением соединения.

**Architecture:** Перенести сервисы из Tauri в библиотеку runtime, оставить временный IPC-адаптер desktop. Создать versioned API, типизированный TS-клиент и React-free модели. Операциями владеет Host, транспорт переносит команды и подписки.

**Tech Stack:** Существующие Rust/Tokio/Cordis/Rig/SQLite, React 19, Vite, assistant-ui, Node.js 24/npm; Axum для HTTP/WS. Новые зависимости фиксируются при реализации в Cargo.lock/package-lock.json.

**Spec:** [Архитектура платформы](../specs/2026-11-07-web-platform-design.md).

**Фактический статус, 2026-11-07:** A1–A3 реализованы; рабочие части A4–A5 проходят Rust/Node и browser E2E с настоящим Host. Полная приёмка всей программы не объявлена: browser-сценарий с tool-result и сменой workspace, Linux hosted integration и остальные пункты B–D остаются отдельно. Подробные результаты и отклонения от плана: [фактический прогресс](../../validation/web-platform-progress.md). Реализация сохранена одним интеграционным checkpoint вместо отдельных коммитов A1–A5.

## Global Constraints

- Серверный режим с первого выпуска поддерживает нескольких пользователей с изоляцией данных и исполнения.
- Один active run на сессию сохраняется.
- Рабочая БД не используется для тестов.
- Прямые бизнес-вызовы Tauri в целевом web-клиенте запрещены; временный desktop IPC живёт только в адаптере.
- Сохраняются structured history, нейтральный transcript и атомарная запись assistant + integration state.
- Этап A доступен только на loopback с локальной авторизацией. Это внутренний этап, не hosted-релиз.

## A1. Извлечь runtime без изменения поведения

**Files:** создать корневой `Cargo.toml`, `crates/runtime/Cargo.toml`, `crates/runtime/src/{lib,config}.rs`; перенести текущие модули из `src-tauri/src/` в `crates/runtime/src/`, кроме `main.rs`; изменить `src-tauri/Cargo.toml`, `src-tauri/src/main.rs`; перенести history tests вместе с runtime.

**Consumes:** текущие `SessionService::open`, `PluginManager::start`, `Runs::default`.

**Produces:** `Runtime::start(config: RuntimeConfig) -> Result<Runtime, RuntimeError>` и `Runtime::shutdown(&self)`; config содержит явные пути и фиксированный workspace scope. Доступные Tauri-команды временно вызывают тот же Runtime.

```rust
pub struct RuntimeConfig {
    pub tenant_id: String,
    pub workspace_id: String,
    pub data_dir: std::path::PathBuf,
    pub config_dir: std::path::PathBuf,
    pub profile: String,
}
// RuntimeError — enum ошибок bootstrap, конфигурации, БД и lifecycle;
// transport преобразует его в стабильный wire-код без внутренних путей.
```

- [ ] На чистой временной БД зафиксировать baseline текущих тестов, сборки и поведения двух независимых Runtime с разными каталогами.
- [ ] Перенести модули и запустить прежние Rust-тесты через новый crate; устранить зависимости от Tauri в dependency graph runtime.
- [ ] Передать пути из desktop bootstrap; сохранить identifier `ru.oestudio.rig.prototype` и существующие desktop-каталоги.
- [ ] Проверить частичный отказ старта и обратную остановку сервисов; повторный start после shutdown должен освобождать DB lock.
- [ ] Проверить `cargo test -p minerva-runtime`, `cargo check --manifest-path src-tauri/Cargo.toml`, `npm test`, `npm run build`; создать отдельный локальный коммит `refactor: extract headless runtime` после успешных проверок.

Приёмка: текущий desktop работает на общей библиотеке; `cargo tree -p minerva-runtime` не содержит tauri/wry. Обновление Rust workspace не меняет формат БД.

## A2. Создать контракты и клиент вместо invoke в компонентах

**Files:** `crates/contracts/src/{lib,scope,sessions,runs,providers,plugins,error}.rs`, `packages/client/src/{index,types,tauri,http}.ts`, `scripts/generate-contracts.mjs`, `tests/client-contract.test.mjs`; изменить шесть файлов с invoke, `chatAdapter.ts`, `sessionHistory.ts`, существующие тесты и npm workspace config.

**Produces:** интерфейс `MinervaClient`, стабильные ошибки `{code,message,requestId}` и генерируемые DTO. Rust DTO + зарегистрированные endpoint schemas — источник wire-контрактов. Типы Provider/CodexModel/SessionInfo перестают принадлежать компонентам.

```ts
// Форма результата публичного API, без Rust implementation details.
export type ApiResult<T> =
  | { ok: true; value: T }
  | { ok: false; error: { code: string; message: string; requestId: string } };
export type WorkspaceRef = { workspaceId: string };
// userId и tenantId не являются выбираемыми клиентом полномочиями.
```

- [ ] Написать контрактные fixture-тесты для camelCase, nullable полей, enum, неизвестной версии и error result; сначала получить ожидаемые ошибки отсутствующего клиента.
- [ ] Создать schemas/DTO и генератор; `npm run contracts:check` генерирует во временный каталог и сравнивает с committed результатом.
- [ ] Завернуть существующие 22 команды в методы клиента: sessions(6), runs(2), providers(4), codex(5), plugins(2), shell(3). Заменить прямые импорты invoke на переданный клиент.
- [ ] Сохранить смысл тестов истории: UI не пишет assistant; ошибка не становится prompt-текстом; отмена адресует нужный run; инструменты сопоставляются по callId.
- [ ] Проверить запрет `@tauri-apps/*` в web/client-models/plugin SDK, разрешив временный импорт только в `packages/client/src/tauri.ts`; выполнить typecheck, npm tests и коммит `refactor: introduce typed client boundary`.

## A3. Передать владение run серверу

**Files:** `crates/runtime/src/{runs,conversation,session_store,sessions}.rs`, `crates/runtime/src/run_supervisor.rs`, `crates/runtime/tests/run_ownership.rs`; DTO `crates/contracts/src/runs.rs`.

**Produces:** `start_run(scope, StartRunRequest) -> RunHandle`, `get_run(scope, run_id) -> RunSnapshot`, `cancel_run(scope, run_id) -> CancelResult`. Scope создан Host. `RunHandle` содержит runId и sessionId. RunSnapshot содержит status, transcript и ошибку отдельно от model messages.

```json
{
  "sessionId": "s1",
  "requestId": "idempotency-key-1",
  "parentId": null,
  "message": { "id": "u1", "text": "Проверь проект" },
  "model": "api:provider/model",
  "options": {}
}
```

- [ ] Добавить тест: клиент start отключился, runtime завершает fake-provider run и сохраняет результат; тест повторного ключа вызывает provider один раз.
- [ ] Сохранение user input, accepted run и idempotency key выполнять в одной транзакции. Совпавший key + другой payload — conflict. Проверить parent принадлежит сессии и ветке; время приёма устанавливает сервер.
- [ ] Перенести выполнение в supervised Tokio task с owned Arc-зависимостями; исключить borrowed guard, завязанный на request future. Все handles сохраняются до settlement/shutdown.
- [ ] Определить terminal statuses: completed, failed, cancelled, interrupted. При старте восстановить interrupted, не повторять внешние инструменты автоматически.
- [ ] Проверить гонку start/cancel, повторный cancel, два start одной сессии, независимые сессии, shutdown с активным run и ошибку final commit; выполнить Rust tests и коммит `feat: supervise runs independently of clients`.

History adapter теперь загружает историю; создание пользовательского сообщения при отправке принадлежит runs.start. Повторные assistant-ui append не создают вторую запись. Старый сценарий save + run поддерживается только временным IPC bridge, без двойного сохранения в web.

## A4. Поднять API и поток с восстановлением

**Files:** `apps/server/src/{main,local}.rs`, `apps/worker/src/main.rs`, `crates/api/src/{lib,routes,streams,local_auth}.rs`, `crates/runtime/src/events.rs`, `crates/api/tests/{http_api,stream_recovery}.rs`; `packages/client/src/{http,streams}.ts`. HTTP/WS зависимости принадлежат minerva-api, а не minerva-runtime.

**Produces:** `/api/v1/bootstrap`, `/api/v1/rpc`, `/api/v1/streams`, `/health/live`, `/health/ready`. RPC принимает только зарегистрированные method/params, а WS — open/close логического subscription. Используется тот же dispatch и проверка scope для HTTP/WS.

```ts
export type SessionFrame =
  | { kind: 'baseline'; sessionId: string; cursor: number; messages: unknown[]; activeRun: unknown | null }
  | { kind: 'event'; sessionId: string; seq: number; type: string; data: unknown }
  | { kind: 'live'; sessionId: string; runId: string; seq: number; parts: unknown[] }
  | { kind: 'resync'; sessionId: string };
// unknown здесь — граница decoder; бизнес-код получает валидированные DTO A2.
```

- [ ] Написать HTTP-тесты реального listener: отсутствие auth, неверный Origin/Host, неизвестный метод, неверный DTO, unsupported protocol и ожидаемый success.
- [ ] Сделать local bootstrap с одноразовым credential: передача из desktop по приватному каналу, для CLI browser launch — одноразовый fragment с немедленной очисткой и обменом на HttpOnly cookie; не писать token в access logs. Проверять Host/Origin и CSRF на mutating RPC.
- [ ] Ввести monotonic durable seq и атомарную запись event + state. Начать с user accepted, run status, transcript checkpoint и settlement; существующую историю импортировать snapshot-событием.
- [ ] Реализовать baseline+tail без гонки: зарегистрировать подписку, прочитать snapshot/cursor, отбросить buffered events до cursor, затем отдать непрерывный хвост. Проверить границы транзакций и backpressure.
- [ ] Добавить live snapshots и stream reconnect с заменой baseline. Disconnect отменяет подписку, но не run. На logout/отзыв прав stream закрывается.
- [ ] Прогнать HTTP/WS тесты с потерей соединения до/после commit и сменой generation; выполнить `cargo test -p minerva-api -p minerva-runtime` и коммит `feat: expose versioned web api and session streams`.

Генерация token, cookie policy и проверки Origin реализуются библиотечными средствами; local auth нельзя включать на внешнем bind.

## A5. Выделить web-приложение и клиентские модели

**Files:** перенести frontend в `apps/web/`; создать `apps/web/src/app/{bootstrap,App}.tsx`, `apps/web/src/pages/{chat,settings,plugins}/`, `packages/client-models/src/{sessions,runs,providers,plugins}.ts`, `apps/web/vite.config.ts`, `tests/web-runtime.spec.ts`; обновить npm scripts, TS paths, Tauri build paths и импорты прежних тестов.

**Produces:** модели с `getSnapshot()`, `subscribe(listener)` и типизированными actions. UI не восстанавливает transport state своими useEffect. assistant-ui остаётся адаптером представления.

- [ ] Написать model-тест задержанного list response после нового stream event: старый ответ не откатывает новое состояние. Отдельно проверить замену baseline после reconnect.
- [ ] Разделить нынешний main.tsx на bootstrap, приложение и страницу чата; вынести каталог моделей и настройки в модели.
- [ ] Настроить Vite proxy `/api` с WS, production static serving и SPA fallback только для UI routes. API/несуществующий plugin asset не должны возвращать index.html.
- [ ] Namespace browser preferences по server/principal/workspace; logout и переключение workspace очищают чувствительные модели и подписки.
- [ ] Добавить browser E2E с настоящим Rust Host и временной БД: создать сессию, fake-provider ответ с tool-result, reload, disconnect/reconnect, cancel, смена workspace. IPC mock не заменяет этот тест.
- [ ] Выполнить `npm test`, `npm run build`, `npm run test:e2e:web`, Rust tests; коммит `feat: run shared frontend against headless host`.

## Gate A

Записать в `docs/validation/web-platform-a.md` фактические команды/версии/результаты. UI работает в браузере без Tauri; старый desktop ещё доступен через временный адаптер. Все временные данные удаляются только после закрытия test Host. Публичный bind и hosted-режим остаются недоступны до [этапа B](2026-11-07-b-hosted-isolation.md).
