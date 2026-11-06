# B. Многопользовательский сервер — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Делегирование применяется только при отдельном разрешении пользователя.

**Goal:** Дать нескольким пользователям доступ через сайт с изоляцией истории, ключей, файлов, процессов и установленных расширений.

**Architecture:** Публичный Gateway проверяет identity и ресурсы; каждому workspace назначается один private Host. Отдельный supervisor управляет sandbox-окружениями и не выполняет команды от браузера напрямую. PostgreSQL хранит управление, SQLite workspace — историю.

**Tech Stack:** Rust/Axum/Tokio, PostgreSQL, OIDC authorization code + PKCE, OCI/runsc, HTTPS reverse proxy; существующий runtime A.

**Spec:** [Архитектура платформы](../specs/2026-11-07-web-platform-design.md).

## Global Constraints

- Серверный режим с первого выпуска поддерживает нескольких пользователей с изоляцией данных и исполнения.
- Серверный выпуск рассчитан на Linux x86_64.
- Platform administrator отдельно управляет каталогом и инфраструктурой.
- Рабочая БД не используется для тестов.
- Этап B не завершён по одним unit-тестам: требуется реальное Linux/runsc окружение.

## B1. Identity и tenancy

**Files:** `crates/server-control/src/{lib,identity,oidc,membership}.rs`, `crates/server-control/migrations/001_identity.sql`, `apps/server/src/hosted.rs`, `apps/web/src/pages/login/`, `tests/hosted/auth.spec.ts`.

**Consumes:** API bootstrap и scoped runtime A.

**Produces:** `Principal { user_id, tenant_id }` после аутентификации; `AuthorizedWorkspace` после проверки membership. Эти типы не десериализуются из browser DTO.

```sql
CREATE TABLE tenants (id uuid PRIMARY KEY);
CREATE TABLE users (
  id uuid PRIMARY KEY, issuer text NOT NULL, subject text NOT NULL,
  UNIQUE (issuer, subject)
);
CREATE TABLE workspaces (
  id uuid PRIMARY KEY, tenant_id uuid NOT NULL REFERENCES tenants(id),
  UNIQUE (tenant_id, id)
);
CREATE TABLE workspace_memberships (
  user_id uuid NOT NULL REFERENCES users(id), tenant_id uuid NOT NULL,
  workspace_id uuid NOT NULL, role text NOT NULL
    CHECK (role IN ('owner','editor','viewer')),
  PRIMARY KEY (user_id, tenant_id, workspace_id),
  FOREIGN KEY (tenant_id, workspace_id) REFERENCES workspaces(tenant_id,id)
);
```

- [ ] Создать integration fixture с двумя OIDC identities и независимыми workspace; проверить уникальность issuer/subject и невозможность смешать tenant/workspace.
- [ ] Реализовать OIDC code flow через backend, state/nonce/PKCE, проверку issuer/audience/signature/expiry. Browser получает только HttpOnly Secure session cookie.
- [ ] При первом входе транзакционно создать личный tenant/workspace/owner membership; повторный callback не создаёт дубликаты. Политика допуска новых identities задаётся администратором.
- [ ] Реализовать logout/revocation, CSRF, ограниченные redirect URL и срок сессии. Права platform admin не выводятся из workspace owner или произвольного email.
- [ ] Проверить replay callback, неверные claims, истёкшую сессию и отсутствие токенов в логах; выполнить `cargo test -p minerva-server-control --test identity`, browser auth test и коммит `feat: add hosted identity and workspace ownership`.

## B2. Единая авторизация всех поверхностей

**Files:** `crates/server-control/src/{policy,resources,dispatch,audit}.rs`, `apps/server/src/{rpc,streams,assets}.rs`, `tests/hosted/resource-isolation.spec.ts`.

**Produces:** `authorize(principal, action, resources) -> AuthorizedWorkspace` для вызова, stream open и fetch. Каждый endpoint имеет resource resolver; отсутствие resolver у resource-bearing метода — ошибка регистрации.

```json
{
  "action": "runs.cancel",
  "resource": { "workspaceId": "wa", "sessionId": "sa", "runId": "ra" }
}
```

- [ ] Составить тестовую матрицу owner/editor/viewer × list/read/start/cancel/config/plugins/files; явно проверить Alice не может передать Bob workspace/session/run ID.
- [ ] Применить deny-by-default к RPC, спискам, поиску, WS events, plugin assets, config, файлам, логам и download. Для отсутствующего и чужого объекта использовать одинаковый публичный not-found ответ там, где иначе раскрывается существование.
- [ ] Добавить разрешения на каждый WS scope и прекращение уже открытых потоков при отзыве membership. Переключение вкладки на другой workspace не оставляет прежний stream в модели.
- [ ] Подписывать внутренний scope для конкретного worker assignment с коротким expiry; worker проверяет audience, workspace и method. Не пересылать произвольные browser auth headers как доверенные.
- [ ] Добавить audit событий изменения прав, plugin install/activation, доступа к секретам и отказов; редактировать секреты/содержимое prompts из логов по умолчанию.
- [ ] Выполнить `npm run test:e2e:hosted -- resource-isolation`, Rust policy tests и коммит `feat: enforce resource authorization across transports`.

## B3. Изолированный запуск Host и execution

**Files:** `apps/runner/src/{main,assignments,sandbox,limits}.rs`, `crates/execution/src/{lib,local,remote,policy}.rs`, `crates/server-control/src/assignments.rs`, `profiles/hosted-worker.json`, `tests/hosted/sandbox-isolation.spec.ts`.

**Produces:** `ExecutionProvider` для fs/subprocess, `RunnerControl` для start/stop/status workspace Host. RunnerControl принимает проверенные IDs, image digest и ограниченный launch spec; не произвольную host shell-строку.

```json
{
  "workspaceId": "wa", "generation": 7,
  "profile": "hosted-worker",
  "limits": { "memoryMiB": 2048, "cpuMilli": 1000, "pids": 256, "diskMiB": 10240 }
}
```

- [ ] На Linux проверить runsc с fake worker, реальным shell и бинарником примера; несовместимость должна блокировать hosted readiness, а не включать local fallback.
- [ ] Создать раздельные Host data/config и execution workspace/tmp volumes. Worker не получает platform secrets, другие volumes, Docker socket или host network/PID namespace.
- [ ] Заменить прямой process/filesystem доступ shell через ExecutionProvider; процессное дерево завершать при timeout/cancel/stop. Проверить дочерний процесс, переживающий родительский shell.
- [ ] Ограничить CPU/RAM/PID/disk и сеть для workspace; попытки читать metadata/private control network должны блокироваться на сетевом уровне и в endpoint policy.
- [ ] Свести assignment к единственному владельцу: generation/lease в control DB, запрет маршрутизации старому Host, stop-and-confirm перед новым mount. Проверить конкурирующие старты, сбой gateway и перезапуск runner.
- [ ] Прогнать реальные проверки чтения соседнего sentinel-файла, обхода через symlink, process enumeration, ресурсного истощения, redirect/DNS endpoint attack; соседний workspace остаётся доступен. Коммит `feat: isolate workspace execution and worker ownership`.

Лимиты из примера — стартовые configurable значения, а не заявленная ёмкость сервера. Провести измерение памяти idle/active Host до определения допустимого количества пользователей.

## B4. Scoped credentials и интеграции

**Files:** `crates/server-control/src/{credentials,credential_crypto}.rs`, migration `002_credentials.sql`, `crates/runtime/src/{providers,codex}.rs`, `crates/runtime/src/credential_store.rs`, `tests/hosted/credentials.spec.ts`.

**Produces:** CredentialStore с `put(scope, name, secret)`, `has(scope, reference)`, `delete(scope, reference)` и внутренним `resolve(grant, reference)`. resolve не имеет публичного browser endpoint.

- [ ] Проверить одинаковый provider.id у Alice/Bob: независимые ключи, hasKey и удаление. Подмена reference не выдаёт чужой ключ и не отправляет его на чужой endpoint.
- [ ] Реализовать encrypted-at-rest store проверенной AEAD-библиотекой; AAD связывает tenant/workspace/reference/version. Master key передаётся только credential service через серверный secret mount. Проверить rotation и восстановление backup вместе с отдельным ключом.
- [ ] Выдавать только разрешённый секрет конкретному адаптеру; не копировать platform env в Host/commands. Отключить общий environment fallback для hosted.
- [ ] Для API discovery/chat применять одну endpoint policy с контролем redirect/DNS. Смена endpoint не переиспользует ключ без явного действия, как в текущем desktop-инварианте.
- [ ] Изолировать Codex home и lifecycle; проверить существующий клиент и официально поддерживаемый auth flow на сервере. Если подходящий flow недоступен, bootstrap возвращает `codex.login: false` с объяснением, а UI не показывает неработающий вход.
- [ ] Выполнить credential tests, negative endpoint tests и коммит `feat: scope credentials and integration state`.

## B5. Приёмка двух пользователей

**Files:** `tests/hosted/two-users.spec.ts`, `tests/hosted/fixtures.ts`, `deploy/compose.test.yml`, `docs/validation/web-platform-b.md`.

**Fixture contract:** тест создаёт isolated deployment с OIDC test issuer, PostgreSQL, gateway, runner и двумя real sandboxes; выдаёт два browser contexts и функции чтения результата только через API. Fixture cleanup знает только созданные test volume IDs.

- [ ] Alice/Bob одновременно создают sessions и запускают fake provider с различимыми ответами; сравнить API, UI, WS и файлы.
- [ ] Перезагрузить gateway во время run: Host заканчивает работу, после входа виден один результат. Убить Host во время tool: восстановление показывает interrupted и не повторяет tool.
- [ ] Отозвать Bob membership при открытой вкладке и активной подписке; следующие события/скачивания/команды недоступны.
- [ ] Проверить всплеск запросов/процессов одного workspace: действуют квоты и backpressure, второй продолжает работать.
- [ ] Выполнить `npm run test:e2e:hosted` на Linux, записать версии runsc/ядра/образов и все результаты; коммит `test: verify hosted multi-user isolation`.

## Gate B

Все отрицательные проверки доступа и реальные sandbox-тесты проходят. До этого внешняя публикация выключена. Предположения о native plugin trust и UI-origin сохраняются в [этапе C](2026-11-07-c-plugin-platform.md); tenant plugins не получают доступ к control plane.
