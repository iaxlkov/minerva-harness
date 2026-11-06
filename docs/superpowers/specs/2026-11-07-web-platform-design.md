# Minerva: веб-платформа, изоляция пользователей и плагины

Дата: 2026-11-07. Статус: реализация начата; [фактический прогресс и оставшаяся работа](../../validation/web-platform-progress.md). Документ описывает целевую архитектуру, а не утверждает готовность всех этапов.

## Цель и согласованные требования

Один интерфейс Minerva работает в браузере и Tauri. Серверный режим с первого выпуска поддерживает нескольких пользователей с изоляцией данных и исполнения. Плагины расширяют и заменяют интерфейс и backend-функциональность. Архитектурная основа — решения DeepSeek Harness (DSH), адаптированные к нашему Rust + React приложению.

Серверный выпуск рассчитан на Linux x86_64. Desktop сохраняет Windows и macOS. Первый серверный выпуск — один узел исполнения и несколько пользователей; горизонтальное масштабирование не является условием его готовности. Вход пользователей — через OIDC, без собственной системы паролей. Каждый новый пользователь получает личный tenant и workspace. Совместная работа внутри tenant допускается моделью membership и ролями, но приглашения и организации не обязательны для первого выпуска.

Плагины собственного продукта и проверенные администратором расширения получают полный набор продуктовых точек расширения. Произвольный загруженный пользователем JavaScript не выполняется в основном origin приложения. Установка внешних пакетов на сервере — административное действие; пользователь управляет включением доступных ему плагинов внутри workspace.

## Источники и границы заимствования

Документы DSH прочитаны 2026-11-07. В таблице приведены пути и SHA содержимого файлов GitHub; это SHA blob, а не commit всего репозитория. Перед реализацией зависимости фиксируются в lock-файлах, совместимость не предполагается по совпадению названий Cordis.

| Источник | Используемое решение | SHA blob |
|---|---|---|
| [Architecture](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/architecture.md) | Композиция профилей; сервисные контракты, провайдеры и потребители; desktop как носитель общего web runtime | `446c50e5bd3d0dcd6fc245f60f342d364785adfd` |
| [Web Client](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/web-client.md) | Host владеет состоянием, клиентские модели независимы от React, UI собирается плагинами | `93f2fbdf1d74851111c21e5206f78d3dc794a6bf` |
| [Slots](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/subsystems/slots.md) | Типизированные места расширения и регистрации с управляемым жизненным циклом | `d13357d0b28cf5fa592781f52bc4950b02ad0995` |
| [API Gateway](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/api/gateway/README.md) | Типизированный вызов сервисов, доступ по ресурсам, отдельные логические потоки | `fea754c891949a150be1c68fac632101274f042e` |
| [Remote Access](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/api/remote-access/README.md) | Проверки методов и ресурсов; при необходимости изоляции image store документ требует отдельных Host и DSH_HOME | `f888679c15779651caba05f3ed71a7c3ea876bec` |
| [Client modules](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/modules/README.md) | Host выбирает активные browser-модули и отдаёт граф загрузки | `3cf748d9a05d6561b66aae833baee1b6d6ca9dd3` |
| [Plugin UI cookbook](https://github.com/deepseek-ai/deepseek-harness/blob/master/docs/cookbook/adding-a-settings-card.md) | Связанный пакет с Host- и browser-частями, вклад в чужие UI-слоты | `3e473d8bbc52d0bbfddcabc2512dbef1b23a8fb4` |

Из DSH переносим принципы композиции и границы, а не весь исходный код и не его TypeScript-only генератор Remote. Rust-контракты и сетевой протокол Minerva имеют собственную версию. Наличие Cordis realm или проверки Remote само по себе не означает изоляцию процессов и ОС. [Документация безопасности DSH](https://github.com/deepseek-ai/deepseek-harness/blob/master/SAFETY.md) также не представляет проект как готовую защиту для недоверенных нагрузок.

## Текущее состояние Minerva

- `src/main.tsx` совмещает bootstrap, UI чата, сессии, каталог моделей и состояние настроек.
- Прямые импорты Tauri есть в `main.tsx`, `sessionHistory.ts`, `Settings.tsx`, `CodexSettings.tsx`, `Plugins.tsx`, `Shell.tsx`.
- `chatAdapter.ts` уже принимает транспорт параметром. `sessionHistory.ts` умеет принимать подмену IPC, но по умолчанию импортирует Tauri.
- `src-tauri/src/main.rs` содержит 22 Tauri-команды, создание сервисов, пути данных, окно и tray.
- Сервисы sessions/conversation/runs и адаптеры Rig/Codex выделены в модули. Переносить их можно без переписывания продуктовой логики.
- В `plugins.rs` уже работает Cordis lifecycle, но список плагинов и доступ к сервисам фиксированы; переключается только example. UI-плагины и внешняя установка отсутствуют.
- SQLite v4 содержит сессии, структурированные сообщения и состояние запусков. Один экземпляр держит файловую блокировку БД.
- Ключи провайдеров привязаны к `provider.id` в OS keyring; fallback читает общие переменные окружения. Такая область видимости не подходит серверу с несколькими пользователями.
- Run пока связан с future вызвавшей команды. HTTP-обёртка сама по себе не даст независимого от соединения исполнения.

## Решение: общий runtime и отдельный сервер управления

```text
Browser / Tauri WebView
  └─ Web kernel + Client plugins + Models + Typed client
       └─ HTTPS: команды / WebSocket: подписки и уведомления
            └─ Gateway: identity, membership, policy, limits, routing
                 └─ Workspace Host A ─ собственные SQLite, настройки, плагины
                 └─ Workspace Host B ─ собственные SQLite, настройки, плагины
                      └─ Execution provider: изолированная среда команд

Control database: users, tenants, membership, workspaces, assignments, audit
Credential service: изолированные ссылки на секреты и выдача по полномочиям
Runner supervisor: создание, остановка и ограничение Host/Execution окружений
```

**Web kernel** создаёт клиентскую композицию и рендерит корневой слот. **Workspace Host** владеет историей, активными запусками, Cordis-сервисами и конфигурацией одного workspace. **Gateway** аутентифицирует пользователя, проверяет ресурс и направляет запрос в назначенный Host. **Supervisor** запускает изолированные окружения; его API не доступен браузеру и плагинам.

Локальный профиль использует ту же композицию одного Host, локального principal и транспорта на loopback. В desktop Tauri управляет окном, tray и дочерним процессом. Бизнес-команды через Tauri IPC в целевой архитектуре отсутствуют. Подключение к удалённому серверу не запускает локальное ядро и не выдаёт удалённому UI нативные привилегии Tauri.

Доверенный bootstrap безопасности находится за пределами tenant-плагинов: аутентификация, межпользовательская авторизация, назначение volumes и пределы ресурсов не могут быть выключены пользовательским профилем. Продуктовые функции, включая layout, модели, инструменты, контекст и движок исполнения, подключаются через сервисы и плагины.

## Структура репозитория

```text
apps/web/                         browser bootstrap, страницы и композиция
apps/server/                      hosted gateway и локальный HTTP launcher
apps/worker/                      headless Workspace Host
apps/runner/                      доверенный supervisor Linux
apps/desktop/                     Tauri и упаковка локального Host
crates/contracts/                 DTO, ошибки, версии, схемы wire
crates/api/                       HTTP/WS адаптер runtime, без UI и Tauri
crates/runtime/                   bootstrap, профили, run ownership
crates/session/                   история, события, SQLite и миграции
crates/plugin-sdk/                Rust-контракты сервисов/плагинов
crates/plugin-host/               каталог, lifecycle, процессный RPC bridge
crates/server-control/            identity, policy, tenancy, credentials
crates/execution/                 filesystem/subprocess и среды исполнения
plugins/providers-rig/            встроенный Rust-провайдер
plugins/codex/                    встроенный Rust-адаптер
plugins/shell/                    инструмент и настройки shell
plugins/review-example/           устанавливаемый Rust+UI пример
packages/client/                 HTTP/WS клиент и сгенерированные DTO
packages/client-models/           React-free проекции и команды
packages/plugin-sdk/              browser plugin lifecycle, slots, services
packages/ui/                      общие визуальные примитивы
profiles/                        local, hosted-worker, desktop
deploy/                          контейнеры, proxy, OIDC, deployment guide
tests/                           контрактные и сквозные проверки
```

Сначала `crates/runtime` может содержать перенесённые существующие модули. Выделение `session` и встроенных plugins выполняется при введении контрактов; не требуется массово создавать пустые crates. В web приложении используем `app/pages/shared`, а самостоятельные продуктовые расширения размещаем в plugin-пакетах. Фронтенд не получает глобальный mutable runtime через React context.

## Пользователи, ресурсы и изоляция

Идентичность: `UserId` связан с парой OIDC `(issuer, subject)`. `TenantId` — граница организации данных. `WorkspaceId` принадлежит ровно одному tenant. Сессия, run, файл, credential reference и plugin activation имеют единственного владельца workspace либо tenant. Сервер проверяет принадлежность на каждой операции, загрузке файла и подписке. Смена владельца через DTO запрещена.

Роли workspace: `owner` управляет участниками, настройками и активацией плагинов; `editor` читает и запускает задачи; `viewer` только читает разрешённые данные. Platform administrator отдельно управляет каталогом и инфраструктурой. Роль owner workspace не означает owner платформы. В UI предоставляется снимок прав, но окончательное решение принимает сервер.

Контекст вызова создаётся сервером из сессии входа и проверенной membership. Worker получает короткоживущее подписанное утверждение с user/tenant/workspace/action/expiry и своим идентификатором назначения. Прямые запросы к worker извне закрыты. Отзыв доступа прекращает соответствующие потоки; идентификатор объекта, UUID или имя канала не заменяют авторизацию.

На сервере один активный Host обслуживает один workspace. У него отдельные data/config/home/tmp, SQLite и plugin state. Supervisor не монтирует соседние workspace или управляющие сокеты. Для команд и внешних плагинов используются отдельные execution-окружения с выданными mount и сетью; SQLite Host и хранилище секретов туда не монтируются. `HOME`, каталог Codex и окружение процесса не наследуются от администратора сервера.

Для Linux первого hosted-выпуска выбираем OCI-окружения с gVisor/runsc, лимитами CPU/RAM/PID/disk и ограничением исходящей сети. Это проектное решение; [gVisor](https://gvisor.dev/docs/) предоставляет runtime `runsc`, но совместимость наших инструментов должна пройти отдельную проверку. Hosted-профиль не переключается автоматически на обычный локальный процесс при отсутствии sandbox. У локального доверенного desktop-профиля свой execution provider.

Пользовательские endpoint моделей также подчиняются сетевой политике: закрыть доступ к metadata, управляющей сети и loopback сервера, проверять DNS/redirect и actual destination. Доступ к частным endpoint разрешается только политикой администратора. Это относится к discover, chat, plugin HTTP и download.

## Хранилища и секреты

Control plane использует PostgreSQL для identity, membership, назначения Host, каталога пакетов, секретов в зашифрованном виде и audit. История остаётся в отдельной SQLite каждого workspace: это позволяет сохранить текущие транзакции и миграции. Механизм назначения Host + файловая блокировка исключают двух писателей. Новый Host не запускается до подтверждённой остановки старого и освобождения volume; потеря lease делает старый Host недоступным для новых операций.

Ключи шифрования control plane не передаются в workers. Credential service выдаёт конкретный секрет доверенному адаптеру в разрешённой области; API в браузер возвращает только reference и признак наличия. Используется проверенная AEAD-библиотека и версия ключа, без собственной криптографии. Аудит не содержит значений секретов. На desktop реализация CredentialStore использует OS keyring; на сервере общие `OPENAI_API_KEY` и аналогичные переменные не служат fallback для пользователей.

Codex получает отдельное состояние авторизации пользователя/workspace. До подтверждения поддерживаемого удалённого flow capability `codex.login` не объявляется в hosted-профиле; API-провайдеры позволяют пользоваться сервером независимо от этого. Поддержку Codex в hosted нельзя объявлять по одному успешному desktop-login.

При переносе локальной БД она явно импортируется в выбранный workspace владельца. Старые ID и структурированная история сохраняются. Рабочая БД не используется для тестов. Перед миграцией делается согласованная backup-копия; восстановление проверяется отдельно от обновления схемы.

## Протокол и выполнение

- API имеет версию `/api/v1` и типизированные namespace: sessions, runs, providers, plugins, shell, workspaces. Схемы Rust DTO генерируют TypeScript-типы; wire совместимость проверяется fixture-тестами. Строковый вызов произвольной Rust-функции не является API.
- Bootstrap сообщает protocolVersion, principal, workspace, capabilities и ревизию plugin graph. Сам граф загружается после авторизации и проверки workspace.
- Запуск — короткая команда `runs.start`, которая сохраняет вход и idempotency key и возвращает runId. Исполнением владеет supervisor внутри Workspace Host, а не HTTP future. Один active run на сессию сохраняется.
- Повтор start с тем же ключом и тем же payload возвращает прежний runId; другой payload даёт conflict. Не обещаем exactly-once внешние эффекты при падении между инструментом и записью результата. Такой run становится interrupted и автоматически не переисполняется.
- `runs.cancel` — явная команда с проверкой области и runId. Закрытие страницы отписывает читателя. Перезапуск Host сохраняет диагностируемый interrupted-результат и уже записанные действия.
- Действие, требующее подтверждения, переводит run в `waiting_approval`. Решение связано с user/workspace/run/toolCall и точным digest аргументов; ответ на изменённые аргументы не является разрешением. После disconnect ожидание восстанавливается из baseline; после рестарта Host прежнее разрешение автоматически не переиспользуется.
- WebSocket переносит независимые потоки session.follow, run.follow, plugins.follow. Открытие каждого даёт baseline, затем обновления. Авторизация обязательна для каждого scope, не только handshake.
- Durable-события получают монотонный `seq` внутри сессии и сохраняются атомарно с изменением состояния. Live-дельты имеют runId и собственную sequence; они не выдаются за durable историю. Финальный transcript заменяет live-сборку.
- При reconnect модель получает baseline и cursor в согласованной точке, затем хвост событий без окна потерь. Отсутствующие live-дельты заменяются снимком активного run, не запускают модель заново. Медленный читатель получает bounded buffer и resync, а не неограниченную очередь.
- Сохраняем сообщения v4 и добавляем durable events постепенно. Исторические записи импортируются как снимки; прошлые turn/step-события не выдумываются. Projection можно восстановить из snapshot + последующих событий.

Для stream ownership, клиентских моделей и слот-композиции следуем DSH; конкретное wire framing Minerva не обязано совпадать с DSH.

## Контракт плагина

Пакет содержит manifest с `id`, `version`, `apiVersion`, зависимостями, схемой config, requested capabilities, backend entry и необязательным UI entry. Backend и UI поставляются одной согласованной версией и фиксируются digest в lock профиля. Backend-only и UI-only пакеты допустимы.

**Встроенные Rust-плагины:** компилируемые crates, публикующие сервисы в Cordis. Существующие providers, codex, shell переводятся на этот контракт. Точки замены: model provider, execution engine, tool registry, prompt/context transforms, session projections, storage provider. Регистрации имеют владельца, зависимости и cleanup; дубликат единственного provider отклоняется при проверке профиля.

**Устанавливаемые Rust-плагины:** отдельные бинарники с версионированным JSON-RPC по stdio. Host-bridge публикует их сервисы в локальном Cordis и транслирует DTO/события. Плагин не получает Rust-ссылки, общий allocator или доступ к памяти Host. Такой формат не требует стабильного native Rust ABI, которого [Rust Reference](https://doc.rust-lang.org/reference/items/external-blocks.html#abi) не гарантирует. Process boundary сама по себе не sandbox; в hosted запуск выполняет supervisor с отдельным execution scope.

**UI-плагины:** JS/TS+React bundle, browser service и регистрации слотов. Rust-часть выполняет backend-работу; React-компоненты выполняются в браузере. Переход интерфейса на Rust/WASM для этого не требуется. В browser SDK используем Cordis lifecycle по модели DSH; проверяем доступную версию JS Cordis и не считаем её Rust-совместимой по wire автоматически.

Начальные слоты: `root.layout` (single), `navigation.items` (list), `settings.pages` (keyed), `conversation.views` (keyed), `composer.actions` (list), `message.renderers` (keyed), `tool.renderers` (keyed), `workspace.panels` (list). Замены single/keyed определяются явным выбором в профиле; неопределённые конфликты не решаются случайным порядком загрузки. Смена layout и renderer допускается без доступа к реализациям соседнего плагина.

Компоненты получают данные, actions и scoped hooks от UI-адаптера. Transport, Cordis context, секреты и сервисные реализации в props не передаются. Общие React/ReactDOM/SDK имеют единственный экземпляр; loader отклоняет несовместимый ABI. В первой версии допускается тонкий Minerva module registry с factory bundles по образцу DSH; копирование полного DSH-loader не является обязательством.

Включение: resolve dependencies → validate version/config/grants → stage artifacts → start backend → publish healthy services → activate UI graph. Отключение: запрет новых вызовов → drain активных потребителей или явная отмена → dispose регистраций → остановка backend → новая ревизия UI graph. Ошибка активации откатывает регистрации. Пакеты с необратимой data migration не объявляются автоматически откатываемыми.

In-process Rust и same-origin React являются доверенным кодом. Capability manifest описывает и контролирует доступ через SDK, но не изолирует произвольный такой код. Сервер допускает только проверенные platform-admin пакеты в эти режимы. Пользовательский непроверенный UI остаётся недоступным до отдельного sandboxed iframe/bridge SDK; непроверенный native backend не подменяет инфраструктуру Host. Поддержка WASM не требуется для первого релиза.

## Профили

`base` включает сессии, модели, реестр инструментов и политики продукта. `local` добавляет локальные credentials/execution и loopback API. `hosted-worker` получает identity scope, scoped credentials и sandbox providers от платформы. `desktop` добавляет desktop carrier к local. Профили фиксируют версии и providers; tenant override не меняет обязательные ограничения платформы.

## Программа реализации и зависимости

| Этап | План | Результат |
|---|---|---|
| A | [Runtime и web-клиент](../plans/2026-11-07-a-runtime-web.md) | Реальный чат в браузере, независимые runs, versioned API и события |
| B | [Многопользовательский сервер](../plans/2026-11-07-b-hosted-isolation.md) | Два пользователя работают одновременно без доступа к чужим данным и исполнению |
| C | [Плагинная платформа](../plans/2026-11-07-c-plugin-platform.md) | Один устанавливаемый Rust+UI плагин расширяет backend и интерфейс без пересборки Host |
| D | [Desktop, выпуск и миграция](../plans/2026-11-07-d-desktop-release.md) | Одна web-сборка работает в Tauri и на сервере; проверены upgrade/restore |

Порядок A → B → C → D. Контракты principal/workspace и plugin versioning определяются уже в A; серверная публикация запрещена до B. A — внутренний проверяемый этап, не изменение требования о нескольких пользователях. C использует границы из B. D не объявляет hosted готовым до общей проверки изоляции и плагинов.

## Критерии завершения всей программы

1. Linux-сервер запускается без Tauri/WebView и обслуживает минимум двух OIDC-пользователей через HTTPS.
2. Подмена session/run/workspace/plugin/file ID, подписки WS, endpoint моделей и ссылки на артефакты не дают доступа к соседнему tenant.
3. Файлы, процессы, ключи, Codex home, настройки и временные данные разделены; worker и plugin не получают управление хостом.
4. Перезагрузка страницы не отменяет run; явная отмена работает; reconnect не дублирует инструмент.
5. Устанавливаемый пример на Rust добавляет backend-команду, инструмент, страницу настроек и renderer результата в браузере и Tauri.
6. Другой UI-плагин заменяет layout/conversation view через публичный слот; смена provider проходит через профиль.
7. Отключение и сбой плагина не оставляют слушателей, процессов или callable endpoints; пользователь соседнего workspace продолжает работу.
8. История v4 открывается после импорта; replay не исполняет инструменты; backup восстанавливается на чистом окружении.
9. Задокументированы ресурсные лимиты, обновление, восстановление, совместимость API/SDK и фактически проверенные возможности Codex.

## Ограничения первого выпуска

Нет обещания совместимости с DSH-пакетами, marketplace, автоматического масштабирования или произвольного untrusted React в основном origin. Пользовательские plugin permissions не могут отменить границы tenant. Linux sandbox и remote Codex проверяются на реальном окружении; моки не доказывают эти свойства.
