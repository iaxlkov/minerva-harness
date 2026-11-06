# Фактический прогресс web-платформы

Дата: 2026-11-07. Ветка: `feat/web-plugin-platform`. Это промежуточный рабочий срез согласованной программы A–D. Публичный многопользовательский релиз не объявлен готовым.

## Рабочий локальный web-срез

- Runtime извлечён из Tauri в `crates/runtime`; сервисы имеют явные пути данных и scope, общий lifecycle и остановку задач.
- Frontend находится в `apps/web`. Прямых импортов Tauri в компонентах нет; временный native transport живёт в `packages/client/src/tauri.ts`.
- DTO генерируются из Rust через `minerva-contracts`/ts-rs; check проверяет точное совпадение с TypeScript.
- HTTP Host поддерживает одноразовый вход, HttpOnly cookie, Origin/Host/CSRF, типизированные команды и приватные worker assertions.
- `runs.start/get/cancel` принимают запрос атомарно и идемпотентно; задача работает независимо от запросившей вкладки. Ошибка не становится текстом модельной истории.
- SQLite v5 сохраняет события в транзакции со сменой состояния. Reconnect начинает с атомарного baseline/cursor; хвост ограничен и поддерживает resync. Старые записи импортируются честным history.snapshot.
- Перед миграцией существующей БД создаётся backup через SQLite backup API, включая committed WAL.
- UI-модели и plugin lifecycle используют observable snapshots. В реальном UI подключены layout, navigation, settings, composer, message/tool renderers, conversation views и workspace panels.
- Внешние доверенные Rust-бинарники устанавливаются локально с manifest/digest и вызываются через bounded stdio JSON-RPC; предусмотрено отключение и завершение дерева процессов.

## Hosted: реализованные части и открытая интеграция

Реализованы PostgreSQL identity/membership/session migrations, OIDC adapter, CSRF, deny-by-default роли, проверка ресурсов, gateway RPC/WS forwarding, short-lived assertions с ключом каждого назначения и replay cache. Есть AEAD credential service и scoped credential adapters. Runner имеет private stdio/library API, runsc-only OCI driver, durable generation fencing, private mounts, ограничения ресурсов и Btrfs quota checks.

Остаются обязательными:

1. HTTP proxy/provisioning bridge runner → private worker, создание/обновление назначения в PostgreSQL, lease/heartbeat и доставка derived worker key. Gateway сейчас отвергает отсутствие готового назначения.
2. Безопасный сетевой broker моделей с DNS/redirect/connected-peer enforcement и подключение credential service к hosted provider. Hosted-провайдеры сейчас возвращают capability unavailable; обычные ключи окружения и OS keyring в этом режиме не используются.
3. Подключение ExecutionProvider к hosted shell/tool path. Ambient shell/Codex и внешние native plugins в worker закрыты до этой интеграции.
4. Настоящие PostgreSQL migrations/constraints, полный browser OIDC code exchange и два пользователя на Linux/runsc с проверкой файлов, процессов, ключей и потоков. Unit-тесты это не доказывают.

В текущем Windows-окружении Docker Linux daemon не доступен: `docker info` не находит `dockerDesktopLinuxEngine`, запуск установленного Docker Desktop не дал рабочего daemon. Linux/runsc/hosted acceptance не выполнялась. Отсутствие sandbox не включает локальный fallback.

## Остаток плагинной программы и desktop

- Полный registry заменяемых backend services/hooks, plugin ToolSet и approvals ещё не подключены.
- Browser SDK и встроенная UI-композиция работают; динамический loader сторонних UI-пакетов/связанный Rust+UI example и транзакционное обновление пакетов остаются этапом C.
- Native local host пока отвергает заявленные dependencies/capabilities, которые не умеет удовлетворить; это не полноценный hosted plugin manager.
- Tauri использует общее ядро и новый supervised run через временный IPC adapter. Переход на thin HTTP carrier, удалённое подключение и перенос desktop-пакета в `apps/desktop` не завершены.
- Полное deployment packaging, explicit import между workspace, общая backup/restore процедура control plane и матрица Windows/macOS/Linux остаются этапом D.

## Проверки

Проверки ниже записываются по фактическому выводу команд. Fixtures используют временные каталоги/БД и локальный HTTP provider; платные запросы и рабочие пользовательские БД не используются.

- Browser E2E с настоящим Rust Host/SQLite: обычный ответ, reload истории, закрытие вкладки во время задачи, восстановление активной задачи в новой вкладке, финальный ответ без второго вызова модели и явная отмена. Добавлена проверка синхронизации модели между двумя вкладками.
- Отдельные тесты runtime: ownership/idempotency, отмена/shutdown, сбой final commit, durable seq/reconnect, rollback без события, миграция и backup.
- Отдельные security fixtures: Origin/CSRF, unsigned/foreign/tampered worker requests, OIDC claim verification, роли и resource scope, AEAD AAD, replay и derived-key isolation.
- PostgreSQL/runsc/live Codex тесты, требующие внешнего окружения, явно ignored; они не считаются passed.

Итоговая проверка 2026-11-07 на Windows x86_64: Node.js 26.2.0, npm 11.13.0, Rust/Cargo 1.98.0.

| Команда | Фактический результат |
|---|---|
| `npm test` | 45 passed, 0 failed |
| `cargo test --workspace` | 105 passed, 0 failed, 7 ignored |
| `cargo clippy --workspace --all-targets -- -D warnings` | passed |
| `cargo fmt --all -- --check` | passed |
| `npm run contracts:check` | passed, generated DTO совпадают |
| `npm run test:e2e:web` | Rust Host и production web build собраны; 1 комплексный Playwright-сценарий passed |
| `npm run plugin:example` | Rust-бинарник собран, готовый каталог создан в `agent-artifacts/review-example-package` |
| `git diff --check` | passed |

Комплексный browser E2E проверил обычный ответ, reload сохранённой истории, синхронизацию выбранной модели в двух вкладках, закрытие вкладки во время run, восстановление активного run в новой вкладке без повторного вызова provider и явную отмену. Provider — локальная HTTP fixture; IPC mocks не используются. Structured tool-result, rollback, сохранность callId и interrupted replay дополнительно проверены отдельными Rust/Node fixtures; этот browser E2E сам инструмент не вызывает и workspace не переключает.

Семь ignored Rust-тестов требуют PostgreSQL, подписанного локального Codex или специальной копии старой БД. Linux/runsc acceptance test дополнительно ограничен `cfg(target_os = "linux")` и на Windows не компилируется. Полный browser OIDC flow, реальная sandbox-изоляция, macOS и Tauri GUI этой проверкой не подтверждены.

Vite сообщает о существующих сторонних `use client` directives и крупном JS chunk (~1 MB до gzip); сборка проходит. При Rust tests MSVC выводит информационные linker messages о создаваемых import libraries. Clippy проходит с `-D warnings`.

Независимые ревью runtime/API, frontend/slots, hosted boundaries и native plugin host завершены. Исправлены найденные проблемы: заголовок принятой сессии, таймауты WS writes, keyed message renderer, синхронизация модели, смена workspace через новый authenticated bootstrap, валидация gateway events, отмена до запуска execution, управление дочерними процессами, reserved plugin IDs и конкурентная запись registry. Повторное scoped review не оставило подтверждённых critical/high/medium замечаний. Это не заменяет перечисленную выше hosted acceptance.
