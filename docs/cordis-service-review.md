# Сверка сервисов с DeepSeek Harness

Дата: 2026-11-07. Исходники DSH проверены на commit
`fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159` в `deepseek-ai/deepseek-harness`.
Это сравнение текущих границ и возможностей, не план копирования всего DSH.

## Как устроена композиция

В DSH типизированный ключ `Context` объявляет контракт; `Service` или
`ctx.provide` публикует реализацию; `inject` определяет условия активации.
Пакет, плагин и сервис — разные сущности. Например, `Session` — обычный класс,
`SessionStore` — сервис, а todo реализован инструментом и проекцией.

[Базовый bundle](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/bundle/base/cordis.patch.yml)
собирает продукт; web/headless/sdk/acp добавляют профильные patches. Запуск
зависит от доступности сервисов, а не порядка строк YAML. В Rust полезны эти
границы владения и явная композиция, перенос YAML/JS expressions не требуется.

У нас `plugins/mh-plugin-manager/src/plugins/builtin.rs` публикует обязательные системные
плагины. Подключения MCP принадлежат системному MCP-плагину; внешний Codex
остаётся установленным процессным пакетом.

## Соответствие

Пути DSH в таблице — относительно `packages/`, наши — относительно корня.
«Нет» означает отсутствие аналога в native runtime; внешний engine может иметь
собственные возможности.

| Область | Сервисы DSH и исходники | У нас |
|---|---|---|
| Инструменты и подтверждения | `ToolsService` (`core/tools`), `ApprovalService`, `UserQuestionService`, `PermissionPresetService` (`interaction/*`) | Есть: `ToolRegistry`, `InteractionBroker`, политики. Публикуются `ToolRuntimePlugin`. |
| Сессии и agent loop | `SessionStore` (`core/session`), `AgentRegistry`, `AgentLoop` (`core/agent-loop`), `SessionProjectionRegistry` | Системные Cordis-плагины `sessions` и `agent-loop` владеют SessionService, активными Runs и supervisor tasks. Runtime собирает сервисы и делегирует вызовы. |
| Модели | `LlmRuntime` (`llm/llm`), default model configuration | Есть `ProviderService`; внешние engines — отдельная граница SDK/host. |
| Workspace, FS, shell | `WorkspaceRegistry`, `WorkingDirectoryService`, `FileSystem`, `ShellExecutor`, `SandboxProvider` (`workspace/*`) | `ShellService` есть; cwd — в `ContextService`, файловые операции — native tools. `WorkspaceTools` обозначает активацию, не заменяемый FS backend. |
| Prompt и skills | `SystemPrompt` (`core/system-prompt`), `SkillRegistry` (`skill/skill`) | Реализованы в `ContextService`; общего API для регистрации prompt sections плагинами нет. |
| Compaction и вложения | `CompactionEngine` (`compaction/compaction`), `TokenMeter`, `ToolResultPruner`, `AttachmentStore` (`attachment/attachment`) | Реализованы вместе в `ContextService` с SQLite. Отсутствие отдельных сервисов не означает отсутствие функций. |
| MCP | `McpServerManager` (`mcp/mcp-manager`), `McpResourceRuntime` (`mcp/mcp-resources`) | Manager и инструменты есть. `resources/list`, `resources/templates/list`, `resources/read` пока нет. |
| Settings | `ConfigEditor` (`boot/config-editor`), `SettingsForms` (`settings/settings`), remote controller, frontend schemas | `minerva.conf.json` задаёт выбор плагинов и параметры shell/agent-loop; атомарное сохранение под writer lock. Формы provider/MCP и внешних плагинов сохраняют свои хранилища; общего schema-driven редактора нет. |
| Remote/UI | `TypertGatewayService` (`api/gateway`), `RemoteAccessService` (`api/remote-access`), slots/session/controllers | Есть собственные client API, browser plugin host и slots. Generated Typert registry нет; transport/auth при этом есть. |
| Subagents/teams/plan/goals | `SubagentRuntime` (`subagent/subagent`), `TeamService`, `PlanModeController` (`plan/plan-mode`), `GoalService` | Native аналоги не обнаружены. Todo в DSH — tool + session projection, отдельный сервис для него не обязателен. |
| Фоновые задачи и расписание | `JobRegistry` (`jobs/jobs`), `jobs-local`, `ScheduleService` (`schedule/schedule`) | Общего registry нет. `Runs` следит за ходами диалога, а не произвольными фоновыми командами. |
| Хранение и большие результаты | `SessionPersistence`, `SessionQueryEngine`, `Storage`, projection cache, `SpillStore` (`spill/spill`) | Есть SQLite persistence, shell spill и truncation инструментов; общего spill/artifact service нет. |

## Что делать следующим

1. **MCP resources** как расширение системного MCP-плагина: discovery,
   templates/read, отмена и отзыв registrations при смене поколения соединения.
   Это конкретный отсутствующий сценарий поверх работающего транспорта.
   [Реализация DSH](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/mcp/mcp-resources/src/index.ts).
2. **Sessions/run lifecycle через Cordis — реализовано:** сохранены SQLite и
   conversation executor; публикацией и завершением владеют системные плагины.
   [Конфигурация композиции](configuration.md) — `minerva.conf.json`.
3. **Prompt contributors**: небольшой owner-scoped реестр секций, если плагины
   должны дополнять инструкции. Делить весь `ContextService` заранее не нужно.
4. **Jobs**, затем subagents/plan: общий lifecycle start/poll/cancel/wait,
   ограниченный вывод и очистка по владельцу. Schedule добавлять поверх jobs,
   когда потребуется выполнение по времени.
5. **Общие settings** при расширении набора форм: revision checks, сохранение
   скрытых секретов, изменение отдельных полей вместо замены всей конфигурации.

## Проверка MCP в desktop

Для ручной/автоматизированной приёмки установлен официальный
`@modelcontextprotocol/server-filesystem@2.2.0` в игнорируемой
`agent-artifacts/mcp-demo`. Сервер `filesystem-demo` работает через stdio с
`--read-only --confine-roots`, доступ ограничен `agent-artifacts/mcp-demo/data`.
Основные npm dependencies проекта не менялись.

Первый реальный запрос выявил несовместимость имён: harness допускает
`mcp.<server>.<tool>`, Codex App Server требует `[a-zA-Z0-9_-]+`. Исправление
находится во внешнем `minerva-codex-plugin`: стабильные SHA256 aliases для
несовместимых/длинных имён, обратное преобразование перед host execution и
сохранением transcript. Исходные имена остаются в описаниях инструментов.
Коллизии aliases отклоняются до запуска. Подтверждения и registry не обходятся.

Регрессионный process smoke перед исправлением падал, после — прошёл и
проверил обратное преобразование `mcp.fixture.read`. Также прошли 14 unit-тестов
(2 opt-in теста пропущены), Clippy all-targets с `-D warnings` и `cargo fmt --check`.

Native UI/chat приёмка прошла с настоящим `plugin:codex/gpt-6-astra`:
сервер виден в настройках с 10 инструментами, один `read_text_file` прошёл
через подтверждение, модель вернула случайный маркер из файла. В SQLite
сохранены `tool-call`, `tool-result` и текст ответа. После перезагрузки
история совпала, карточка осталась `completed`.

Результаты native UI/chat проверки и снимки сохраняются в
`agent-artifacts/mcp-demo/report.json`, `settings.png`, `approval.png`,
`chat.png`, `chat-reloaded.png`. Итог прохождения — поле `passed` в отчёте;
одного состояния `connected` недостаточно для доказательства работы в чате.

Проверка не покрывает весь DSH, remote OAuth MCP, Unix, ресурсы MCP и все модели.
