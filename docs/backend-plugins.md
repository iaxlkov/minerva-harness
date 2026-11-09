# Backend-плагины

Встроенными сервисами управляет Cordis. Внешние пакеты исполняются отдельными процессами через `minerva-plugin-host`. Общий roster менеджера показывает фактическое состояние `enabled`, индивидуальный выбор `requestedEnabled`, глобальный режим `optionalEnabled` и диагностическую ошибку `error`.

## Общий переключатель

Переключатель «Несистемные плагины» находится в строке менеджера плагинов. Он приостанавливает внешние плагины и встроенный тестовый плагин, сохраняя индивидуальный выбор. Включение возвращает только выбранные плагины. Выбор можно менять и во время паузы. Режим и выбор сохраняются в `minerva.conf.json` и восстанавливаются после перезапуска. Формат и миграция описаны в [конфигурации](configuration.md).

Системные плагины входят в бинарник и активируются через Cordis:

| ID | Сервис |
| --- | --- |
| `plugin-manager` | Управление плагинами |
| `providers` | Провайдеры моделей |
| `shell` | Настройки и выполнение команд |
| `tool-runtime` | Реестр инструментов, разрешения, подтверждения и вопросы |
| `context` | Рабочая папка, инструкции/навыки, резюме и вложения |
| `workspace-tools` | Встроенные файловые инструменты, shell и загрузка навыков |
| `mcp` | Подключения и инструменты MCP-серверов |
| `sessions` | SQLite истории, событий и восстановление прерванных запусков |
| `agent-loop` | Admission, активные запуски, отмена и ограниченное ожидание завершения задач |

Общий переключатель их не останавливает, индивидуальное отключение backend отклоняет. MCP-менеджер и его включённые серверы продолжают работать при паузе несистемных плагинов; конкретный сервер отключается своей настройкой `enabled`. В hosted-профиле локальные инструменты и внешние процессы остаются недоступными.

Список и фактическая доступность берутся из опубликованных сервисов Cordis, а не из отдельного флага «системный». При завершении или неудачной активации fibers освобождаются в обратном порядке. Эффекты плагина отзывают встроенные инструменты и останавливают MCP-соединения. Системные ID зарезервированы в SDK, установленный пакет не может занять их или объявить себя системным.

Композиция — `crates/runtime/src/plugins/builtin.rs`. Существующие сервисы и форматы настроек сохранены; `Harness` подготавливает ссылки на ресурсы, а публикацией, запуском и освобождением управляют плагины. Контекстные возможности сгруппированы в один плагин, поскольку используют общее SQLite-хранилище.

HTTP и Tauri предоставляют `set_optional_plugins_enabled({enabled: boolean})`, возвращающий обновлённый roster. В gateway применяется та же политика `ManagePlugins`, что и к индивидуальному переключению.

## Зависимости и сервисы

Провайдер объявляет именованный контракт и его целочисленную версию:

```json
{"provides": {"example.format": 1}}
```

Потребитель объявляет требование:

```json
{"requires": {"example.format": 1}}
```

Оба пакета должны быть выбраны пользователем. Host проверяет наличие единственного провайдера, точное совпадение версии и отсутствие циклов. Требования относятся к сервисам, не к ID пакетов. Неизвестные встроенные wire-контракты (например, `sessions`) не предоставляются автоматически.

Провайдер запускается до потребителя; остановка идёт в обратном порядке. Отключение или падение провайдера останавливает зависимых потребителей, сохраняя их выбор. Их UI освобождается после обновления roster. Повторное включение провайдера восстанавливает выбранных потребителей; прерванные чаты и инструменты автоматически не повторяются.

Во время обработки запроса Host, включая `initialize`, процесс-потребитель может отправить обратный JSON-RPC запрос:

```json
{"jsonrpc":"2.0","id":"format-1","method":"services.call","params":{"service":"example.format","version":1,"method":"format","params":{"text":"hello"}}}
```

Host определяет потребителя по процессу, проверяет его `requires` и отправляет провайдеру `services.invoke` с тем же объектом `params`. Провайдер явно диспетчеризует имя сервиса, версию и метод. Ответ возвращается потребителю с исходным ID. Произвольного доступа к приватным методам провайдера этот маршрут не предоставляет.

Обратные вызовы обрабатываются внутри текущего запроса Host, а не как фоновый поток событий. Сохраняются общий таймаут запроса, ограничение размера фрейма и числа обратных вызовов. Очередь RPC своя у каждого процесса; операции изменения графа сериализуются отдельно. Пакеты остаются доверенным локальным кодом: процессная граница не является sandbox.

## Код

- `crates/plugin-host/src/package.rs` — проверка пакета, digest и установка.
- `crates/plugin-host/src/storage.rs` — lock-файл и атомарные изменения под writer lock.
- `crates/plugin-host/src/process.rs` — транспорт, таймауты и завершение дерева процессов.
- `crates/plugin-host/src/graph.rs` — зависимости и маршрутизация сервисов.
- `crates/plugin-host/src/lib.rs` — состояние и жизненный цикл Host.
- `crates/runtime/src/plugins.rs` — системные сервисы Cordis и общий API менеджера.


## Harness tool contract

Генераторы изображений расширяют общий механизм харнеса через capability `image-generation`; контракт, проверка доступности и выбор пользователя описаны в [image-generation.md](image-generation.md).

Installed packages may declare the `tools` capability. `tools.list` returns SDK
`ToolDefinition[]` (`name`, `description`, `inputSchema`, `outputSchema`,
`sideEffect`). Raw names use ASCII letters, digits and underscores, at most64
bytes. The host adds a stable plugin namespace. Catalogues are admitted per
process generation and revoked on disable, pause, failure or shutdown.

Model calls use short RPCs: `tools.start({runId,callId,name,arguments})`, then
`tools.poll({runId,callId})`, returning `{status,output?,error?}`. Status is
`running`, `completed`, `failed` or `cancelled`. Both `tools.cancel` and
`tools.release` take the same identity and must be idempotent. Long work must
never block the process RPC reader. The host cancels and releases on error,
timeout, cancellation or a dropped model future. Browser calls cannot invoke
these execution methods. Argument/output schemas and approval policy are
checked by the shared registry before effects execute.

External model engines receive optional `engine.start.context` with
`systemPrompt`, `cwd`, `permissionPolicy`, and admitted `tools`. Poll snapshots
may contain `requests: [{id,callId,kind,tool?,arguments?,prompt?,options?}]` while
running. Kinds are `tool`, `approval` and `question`. The host replies through
`engine.answer({runId,id,decision,answer,output,isError})`. Repeated requests
must retain their identity and content. Host tools and answers remain scoped to
the actual session/run/call. Terminal snapshots never execute new requests.

JSON-RPC incoming responses and ordinary outgoing requests are bounded to1MiB.
Only outgoing `engine.start` permits32MiB for validated inline image inputs;
plugins must avoid echoing base64 in poll snapshots. Mutable state belongs in
`initialize.dataDir`, outside the immutable package digest. Native trusted
plugins and explicitly approved shell commands are not an OS security sandbox.
