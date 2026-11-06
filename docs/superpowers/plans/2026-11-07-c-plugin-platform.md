# C. Плагинная платформа — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Делегирование применяется только при отдельном разрешении пользователя.

**Goal:** Подключать один пакет с Rust backend и React UI, расширяющий сервисы, инструменты и интерфейс без пересборки приложения.

**Architecture:** По модели DSH профиль собирает сервисы Host и browser plugin graph; UI компонуется через типизированные слоты. Built-in Rust crates используют Cordis напрямую, устанавливаемые бинарники — versioned process bridge. Права и isolation определяются платформой B.

**Tech Stack:** cordis-rs, browser Cordis lifecycle, Rust + JSON-RPC/stdin/stdout, React slots, manifest JSON Schema, content-addressed bundles. Текущие React/assistant-ui/Rig сохраняются.

**Spec:** [Архитектура платформы](../specs/2026-11-07-web-platform-design.md).

## Global Constraints

- Backend и UI поставляются одной согласованной версией и фиксируются digest в lock профиля.
- Process boundary сама по себе не sandbox; в hosted запуск выполняет supervisor с отдельным execution scope.
- Пользовательские plugin permissions не могут отменить границы tenant.
- Рабочая БД не используется для тестов.
- Совместимость Minerva plugin SDK с пакетами DSH не предполагается.
- Непроверенный same-origin JS не выполняется: server catalog доверен platform administrator.

## C1. Контракты сервисов и профиль вместо фиксированного списка

**Files:** `crates/plugin-sdk/src/{lib,manifest,services,hooks,lifecycle}.rs`, `crates/plugin-host/src/{lib,registry,profile}.rs`, `profiles/{base,local,hosted-worker,desktop}.json`; изменить `crates/runtime/src/{plugins,conversation,config}.rs`; тест `crates/plugin-host/tests/composition.rs`.

**Consumes:** scopes и execution/credential providers из B.

**Produces:** manifest v1, validated composition, named service providers и disposable registrations. Минимальные namespaces сервисов: sessions, models, tools, agent-engine, prompts, execution, credentials. У каждого contract собственная версия, inputs/results schema и правило единственного provider либо keyed registry.

```json
{
  "id": "minerva.review-example",
  "version": "1.0.0",
  "apiVersion": 1,
  "requires": { "sessions": 1, "tools": 1 },
  "capabilities": ["workspace.files.read", "session.events.append"],
  "backend": { "kind": "process", "entry": "bin/review-plugin" },
  "ui": { "entry": "ui/client.js", "sdkVersion": 1 },
  "configSchema": {
    "type": "object", "additionalProperties": false,
    "properties": { "maxFiles": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 } }
  }
}
```

Archive manifest описывает относительные entry внутри проверенного пакета. Registry lock хранит digest всего пакета и разрешённые platform artifacts; manifest не выбирает произвольный executable хоста.

- [ ] Добавить тесты missing dependency, cycle, несовместимой версии, двух provider одного single-service и неразрешённого grant.
- [ ] Сделать profile resolver с порядком dependencies и явной заменой providers. Hosted overrides могут только сузить platform policy; запрет отключать обязательные security providers.
- [ ] Вынести текущие сервисы в `plugins/providers-rig`, `plugins/codex`, `plugins/shell`, session domain — в `crates/session`; обновить импорты без изменения хранения.
- [ ] Заменить `PluginManager::providers()/codex()/shell()` как единственный путь расширения на реестр контрактов. Runtime выбирает AgentEngine по зарегистрированному provider, а не жёсткому условию в UI.
- [ ] Определить hooks с фазами `before_run`, `before_model`, `before_tool`, `after_tool`, `after_run`: вход/результат валидируются, порядок явный, timeout ограничен. Изменения model-visible context и версии plugin config фиксируются в run snapshot.
- [ ] Выполнить `cargo test -p minerva-plugin-host --test composition`, регрессии истории и коммит `feat: compose runtime through plugin contracts`.

Не переносить Rust-объекты через JSON-RPC. Для процессного provider только сериализуемые wire contracts; gateway auth и sandbox supervisor не являются заменяемыми tenant hooks.

## C2. Browser SDK и слоты

**Files:** `packages/plugin-sdk/src/{index,context,services,slots,lifecycle}.ts`, `packages/plugin-sdk/src/renderer.tsx`, `packages/plugin-sdk/src/slots.types.ts`, `apps/web/src/app/plugin-host.ts`, `tests/plugin-slots.test.tsx`.

**Produces:** `BrowserPlugin.apply(ctx)` и scoped registries. Bootstrap единственный получает lifecycle context; presentation получает только props, scoped hooks и actions. Общие UI primitives выносятся в `packages/ui`.

```ts
export type Disposable = { dispose(): void };
export type SlotDeclarations = {
  'root.layout': { kind: 'single'; scope: 'root' };
  'navigation.items': { kind: 'list'; scope: 'workspace' };
  'settings.pages': { kind: 'keyed'; scope: 'workspace' };
  'conversation.views': { kind: 'keyed'; scope: 'session' };
  'composer.actions': { kind: 'list'; scope: 'session' };
  'message.renderers': { kind: 'keyed'; scope: 'session' };
  'tool.renderers': { kind: 'keyed'; scope: 'session' };
  'workspace.panels': { kind: 'list'; scope: 'workspace' };
};
// Каждая registration содержит ownerPluginId, slot, entryId,
// компонент и порядок; registry возвращает Disposable.
```

- [ ] Проверить browser Cordis на минимальном create/apply/dispose сервисе и сверить license/version; зафиксировать зависимость. Не заменять lifecycle произвольными глобальными массивами.
- [ ] Написать тесты регистрации в неизвестный slot, конфликтующей замены, отсутствующего session scope и очистки дочерних регистраций.
- [ ] Реализовать slot registry отдельно от React renderer; ошибки UI ловятся на границе plugin, fallback сохраняет доступ к настройкам/отключению.
- [ ] Перевести штатные chat, navigation, settings, providers и shell UI в первые browser plugins; убрать runtime-import чужих feature-компонентов.
- [ ] Проверить замену layout и conversation view примером через публичный SDK; отмена выбора возвращает стандартную реализацию.
- [ ] Выполнить `npm run test:plugins`, browser E2E и коммит `feat: compose web ui through typed plugin slots`.

## C3. Динамическая загрузка связанного UI-пакета

**Files:** `packages/plugin-sdk/src/{modules,boot-graph}.ts`, `apps/web/src/app/plugin-loader.ts`, `crates/plugin-host/src/{artifacts,client_graph}.rs`, `apps/server/src/plugin_assets.rs`, `scripts/build-plugin.mjs`, `tests/plugin-loader.spec.ts`.

**Produces:** authenticated plugin graph с revision и digest; same-origin immutable URLs только для проверенных активных пакетов. Public cache запрещён для workspace config/graph; общие статические bytes могут дедуплицироваться, но выдача проверяет membership и доступ к пакету.

```json
{
  "revision": 12,
  "entries": [
    { "id": "minerva.review-example", "version": "1.0.0", "digest": "sha256:fixture-digest", "uiPath": "ui/client.js" }
  ]
}
```

`fixture-digest` используется только в unit fixture; production graph допускает полный digest, вычисленный сервером и проверенный при установке.

- [ ] Написать loader tests: общий React singleton, отсутствие undeclared imports, несовместимый SDK, cycle и failed script download.
- [ ] Реализовать factory module format с явными imports `react`, `react-dom`, `@minerva/plugin-sdk`; build делает эти модули external. Запретить eval/Function и CDN dependencies в plugin artifacts первого выпуска.
- [ ] Создавать graph из активной серверной композиции после старта backend. UI не объявляет backend healthy самостоятельно.
- [ ] Ревизии graph активировать атомарно; при смене workspace/auth закрывать старые модели и registrations. Загруженные JS-фабрики не являются unload памяти, но их эффекты обязаны быть disposed.
- [ ] Проверить asset path traversal, неверный digest, отключённый plugin и чужой workspace; конфиг/секреты не попадают в bundle или boot JSON.
- [ ] Выполнить plugin loader tests и коммит `feat: load versioned browser plugin bundles`.

## C4. Устанавливаемый Rust backend и ограниченный bridge

**Files:** `crates/plugin-host/src/process/{mod,protocol,supervisor,bridge}.rs`, `crates/plugin-sdk/src/process.rs`, `apps/runner/src/plugin_sandbox.rs`, `crates/plugin-host/tests/process_protocol.rs`.

**Produces:** line-delimited JSON-RPC 2.0 по stdio. `initialize` обменивает apiVersion/pluginId и разрешённые contracts; `activate`, `deactivate`, `shutdown` управляют lifecycle. Backend→Host calls проходят отдельный broker с выданными grants.

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"apiVersion":1,"pluginId":"minerva.review-example"}}
```

- [ ] Сделать fixture binaries для успешного handshake, wrong version, crash, hanging request, oversize frame и stderr logging. stdout используется только для протокола.
- [ ] Реализовать correlation IDs, cancellation, deadlines, bounded messages/queues и обработку EOF. Max frame = 1 MiB, pending calls = 128, handshake timeout = 10 s; крупные файлы идут через scoped artifact service.
- [ ] Превратить объявленные handlers в Cordis services через bridge; вызовы неизвестного/отозванного метода отклоняются до отправки в процесс.
- [ ] Запускать внешний plugin через supervisor B в отдельном scope с private config/storage и только разрешёнными mounts. Tenant/workspace не меняются входным params.
- [ ] Проверить crash снимает registrations и завершает ожидающие вызовы диагностикой; не перезапускать автоматически неидемпотентный tool. Stop завершает дерево процессов и освобождает scope.
- [ ] Выполнить protocol tests, реальные Linux sandbox tests и коммит `feat: host installable rust plugins over scoped rpc`.

## C5. Установка, обновление и lifecycle

**Files:** `crates/plugin-host/src/{installer,activation,lockfile,storage}.rs`, control migration `003_plugins.sql`, `apps/web/src/pages/plugins/`, `tests/hosted/plugin-lifecycle.spec.ts`.

**Produces:** install → staged → active/failed lifecycle; admin catalog отделён от workspace activation. Изменения config используют expectedRevision, backend config validation и write-only secret references.

- [ ] Проверить archive traversal/absolute paths/symlinks, digest mismatch, oversized archive, несовместимую платформу и повреждённый manifest до выполнения кода.
- [ ] Принимать готовые artifacts; не выполнять install/build scripts на gateway. CI производителя собирает Rust для целевых платформ и browser bundle.
- [ ] Реализовать dependency lock и atomic staging; активировать backend перед UI. Запретить второй update того же workspace до окончания первого.
- [ ] На disable перестать принимать новые вызовы, дождаться активного run либо запросить явную отмену; снять hooks/endpoints/UI и остановить процесс. Ход хранит plugin version/config snapshot.
- [ ] При failed activation восстановить предыдущую согласованную версию. При irreversible data migration потребовать backup/maintenance и не обещать автоматический rollback.
- [ ] Проверить повторный enable/disable не умножает listener, update не смешивает UI/backend versions, пакет не влияет на соседний workspace; коммит `feat: manage plugin packages and activation lifecycle`.

История хранит `pluginId`, `eventVersion` и данные. При отключённом или несовместимом renderer отображается generic read-only карточка; просмотр не требует активировать плагин или повторять инструмент.

## C6. Инструменты плагинов, policy и approvals

**Files:** `crates/plugin-sdk/src/tools.rs`, `crates/runtime/src/approvals.rs`, `plugins/providers-rig/src/tools.rs`, `plugins/shell/src/tool.rs`, `apps/web/src/plugins/approvals/`, `crates/runtime/tests/tool_approval.rs`.

**Produces:** ToolRegistry с input/output schemas, risk declaration и handler; `approval.request` durable event; типизированный `approvals.resolve(approvalId, decision)` endpoint. Серверный контекст связывает решение с workspace/run/toolCall и digest аргументов. Actor передаётся из проверенной сессии входа.

- [ ] Добавить fake-provider тест, действительно вызывающий инструмент через Rig, и проверить порядок policy → approval при необходимости → handler → durable result. Ошибка инструмента сохраняется как tool-result с error, а не как выдуманный успешный текст.
- [ ] Адаптировать registry к Rig ToolSet; shell tool вызывает ExecutionProvider B с рабочей директорией текущего workspace. Custom tool не получает обходной доступ к host filesystem.
- [ ] Проверять grant независимо от approval: пользовательское подтверждение не расширяет ограничения sandbox, роли или egress. Hosted default требует подтверждения команд shell и изменяющих внешнее состояние действий; строго read-only пример C7 можно разрешить политикой.
- [ ] Реализовать waiting_approval и доступную UI-карточку через слот. Решение принимается один раз атомарно; viewer, другой workspace, устаревший run и неверные аргументы отклоняются. Таймаут ожидания 15 минут завершает запрос отказом; cancel run отзывает связанные approvals.
- [ ] Проверить reconnect до ответа, двойной клик, отзыв membership и restart Host. Ни один сценарий не повторяет вызов инструмента; после аварии требуется новый явно запрошенный run.
- [ ] Выполнить `cargo test -p minerva-runtime --test tool_approval`, browser approval tests и коммит `feat: execute plugin tools through scoped policy and approvals`.

## C7. Вертикальный пример Rust + UI

**Files:** `plugins/review-example/{plugin.json,backend/Cargo.toml,backend/src/main.rs,ui/src/index.tsx}`, `plugins/review-example/README.md`, `tests/hosted/plugin-example.spec.ts`, `docs/validation/web-platform-c.md`.

**Example:** инструмент `review.summarizeFiles` получает список файлов через broker read grant, считает количество файлов/строк и возвращает структурированный результат. UI добавляет navigation/page, maxFiles setting, composer action и tool renderer. Инструмент не требует платной модели для проверки; fake Rig provider запрашивает его через реальный ToolSet.

- [ ] Собрать внешний пакет и установить в работающий Host без пересборки самого Host.
- [ ] Проверить command из UI и tool из Rig идут в один контракт через policy/approval; before/after tool и результат сохраняются в истории. Для Codex показывать только фактически доступные integration capabilities, не обещать автоматическое подключение Rust ToolSet.
- [ ] Изменить maxFiles с revision check; следующий run использует новое значение, активный сохраняет свой снимок.
- [ ] Отключить пакет после run: UI исчезает, история читается generic renderer, endpoint больше не callable. Снова включить — нет дубликатов.
- [ ] Установить ту же версию Bob с другой config; Alice не видит его состояние. Провалить процесс Alice — Bob продолжает работу.
- [ ] Выполнить `npm run test:e2e:plugins`, Rust tests и сохранить фактические результаты; коммит `test: prove full-stack plugin extension path`.

## Gate C

Проверены расширение и замена UI, новый backend-service, интеграция инструмента, установка без пересборки Host и полная очистка lifecycle. Собственные плагины используют тот же SDK, что пример. Произвольные native DLL и непроверенный same-origin UI остаются недоступны; это сознательные границы модели, а не скрытое отсутствие plugin support.
