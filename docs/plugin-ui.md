# Внешние UI-модули

UI плагинов следует архитектуре DeepSeek Harness: общий реестр модулей, активация через Cordis, регистрация React-компонентов в слотах и контроллеры данных вне компонентов. Это адаптация подхода к нашему Rust Host, а не совместимость с бинарными пакетами DSH.

## Пакет

```json
{"ui":{"entry":"ui/client.js","sdkVersion":2}}
```

`client.js` — один CommonJS bundle, экспортирующий `sdkVersion = 2` и `apply(ctx)`. Сборка оставляет внешними `react`, `react/jsx-runtime`, `react-dom`, `react-dom/client`, `@minerva/ui`. Эти модули предоставляет приложение, поэтому второй React в пакет не попадает. `@minerva/ui` экспортирует Button и компоненты Select. Импорты других пакетов должны быть включены в bundle.

Host отдаёт код только включённого установленного пакета и проверяет digest. Browser-loader загружает bundle через `plugins.ui`, создаёт Blob-модуль с фабрикой CommonJS и передаёт ей общие зависимости. `eval` не требуется. Локальный HTTP CSP разрешает `script-src 'self' blob:`. Hosted-запуск native-пакетов по-прежнему запрещён.

## Контекст и слоты

### Меню композитора

`composer.menu` — список в области workspace, показанный в меню «+» после
встроенного пункта «Прикрепить файл». Регистрация действует для всех сессий workspace;
компонент получает текущий `sessionId`, `disabled` и `onClose`. Используйте
`ComposerMenuItem` из `@minerva/ui`, чтобы сохранить клавиатурную навигацию и фокус.
`order` задаёт порядок пунктов; отключение плагина удаляет его пункты автоматически.

```tsx
import { ComposerMenuItem } from '@minerva/ui';
export const sdkVersion = 2;
export function apply(ctx) {
  ctx.slots.register('composer.menu', {
    entryId: 'my-plugin:inspect', order: 100,
    component: ({sessionId, disabled, onClose}) => (
      <ComposerMenuItem disabled={disabled} onClick={() => {
        onClose();
        // Open your plugin's view for sessionId here.
      }}>Обработать вложения</ComposerMenuItem>
    ),
  });
}
```

`composer.actions` остаётся отдельным слотом кнопок панели. Меню не требует
замены композитора assistant-ui. О загрузке и инструментах см. [Вложения](attachments.md).

`apply(ctx)` получает контекст текущего workspace:

- `ctx.slots.register(slot, entry)` — зарегистрировать компонент; принадлежность плагину и cleanup контролирует Host.
- `ctx.services` — существующий типизированный реестр сервисов.
- `ctx.effect(cleanup)` — зарегистрировать освобождение ресурсов (функция **не** запускается при регистрации).
- `ctx.rpc(method, params)` — вызов своего backend. `engine.*`, `initialize`, `shutdown` зарезервированы для Host.

Раздел настроек регистрируется в `settings.pages`. Компонент получает `placement: 'navigation' | 'body'`, `selectedPage`, `onSelectPage`. Для navigation он возвращает кнопку, для body — панель. Используйте уникальный `entryId`, например `plugin:my-package`.

Контроллер создаётся в `apply`, а компоненты подписываются через `useSyncExternalStore`. Уход со страницы настроек или переключение раздела размонтирует компонент, но не уничтожает контроллер. Черновик и последний снимок сохраняются. Фоновый опрос должен останавливаться, когда подписчиков нет; незавершённые запросы не должны записывать состояние после disposal.

## Жизненный цикл и доверие

Включение активирует UI один раз. Отключение удаляет слоты, сервисы, listeners и контроллеры; запоздалые загрузки не активируются. Повторное включение читает пакет заново. Ошибки загрузки показываются с кнопкой повторной попытки; ошибки рендера локализованы границей компонента. При исчезновении выбранного раздела настройки возвращаются к моделям.

UI работает в контексте приложения и имеет возможности JavaScript этого контекста. Scoped RPC и список импортов — контракт API, **не защита от вредоносного плагина**. Устанавливать следует доверенные пакеты. Legacy HTML/iframe SDK v1 больше не поддерживается.

В этой версии нет межпакетных импортов, отдельных chunks и горячей замены установленного bundle. Обновление выполняется при остановленном приложении с сохранением `plugins/.data`, затем плагин активируется заново.

## Ошибки backend

`list_plugins` возвращает фактическое состояние `enabled` и отдельное поле `error`. Повреждённый пакет или неудачный запуск не останавливают приложение. На странице плагинов можно повторить запуск или отключить автозапуск неисправного пакета. Общие ошибки чтения конфигурации по-прежнему останавливают запуск явно.

Frontend обновляет список через две секунды после завершения предыдущего запроса. После обнаружения падения процесса его UI освобождается, выбранный раздел настроек возвращается к моделям. При временной ошибке транспорта сохраняется последний список и показывается ошибка; новые запросы не перекрываются.

RPC сериализуются отдельно для каждого процесса. Зависание одного плагина не блокирует вызовы других. Каталоги моделей запрашиваются параллельно и валидируются независимо: ошибочный каталог исключается целиком, остальные остаются доступны. Его диагностическая ошибка очищается после успешного ответа.

## Разделы и компоновка приложения

Оболочка использует shadcn `Sidebar`, `Resizable`, `Sheet`, `Tooltip` текущей темы.
В `@minerva/ui` доступны эти компоненты и `Input`, а также прежние Button/Select/DropdownMenu.

Workspace-слот `shell.sections` (keyed) создаёт самостоятельный раздел. `entryId`
должен быть уникальным, например `reports:overview`. Компонент получает
`{placement, selected, section, onNavigate}` (тип `ShellSectionProps` в SDK):

- `rail`: иконка или DropdownMenu. Используйте `Button` с `aria-label`,
  `aria-current={selected ? 'page' : undefined}` и `className="rail-button"`.
- `title`: название раздела, без интерактивных элементов.
- `sidebar`: содержимое контекстной боковой панели.
- `body`: основная страница раздела.

```tsx
ctx.slots.register('shell.sections', {
  entryId: 'reports:overview', order: 100,
  component: ({placement, selected, onNavigate}) => {
    if (placement === 'title') return 'Отчёты';
    if (placement === 'rail') return <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button variant="ghost" size="icon-lg" className="rail-button"
          aria-label="Отчёты" aria-current={selected ? 'page' : undefined}>
          <ReportsIcon/>
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent side="right">
        <DropdownMenuGroup><DropdownMenuItem
          onSelect={() => onNavigate('reports:overview')}>Открыть отчёты</DropdownMenuItem>
        </DropdownMenuGroup>
      </DropdownMenuContent>
    </DropdownMenu>;
    if (placement === 'sidebar') return <ReportNavigation/>;
    return <ReportsPage/>;
  },
});
```

Все дополнительные зоны — list-слоты в workspace scope, с `order` и автоматическим
удалением регистраций при отключении плагина. Props: `{section, onNavigate}`.

| Слот | Место |
| --- | --- |
| `shell.rail.start` / `shell.rail.end` | Начало / низ колонки иконок |
| `shell.sidebar.header` | Под названием боковой панели |
| `shell.sidebar.content` | После основной навигации раздела |
| `shell.sidebar.footer` | Низ боковой панели |
| `shell.header.actions` | Действия справа в шапке окна |
| `composer.model.menu` | Пункты в конце меню модели (session scope, props `{model, onSelect}`; используйте `DropdownMenuItem` из `@minerva/ui`) |
| `composer.image.tools` | Кнопки в панели редактора прикреплённой картинки (например, «Удалить фон»). Props `{name, render, replace}`: `render()` отдаёт текущую картинку с разметкой (PNG), `replace(blob)` подменяет исходник и сбрасывает разметку |
| `composer.footer` | Строка под полем ввода слева от выбора модели (session scope, props `{model}`) |
| `message.code` | Keyed по языку блока кода ```` ``` ```` в сообщении (workspace scope; все диалекты оболочки — ключ `shell`); props `{code, language, sessionId}`. Терминал регистрирует `shell` с кнопкой «Выполнить» |
| `message.actions` | Кнопки на панели сообщения при наведении (session scope, props `{role}`; внутри доступны хуки assistant-ui для текущего сообщения) |
| `shell.canvas.actions` | Пункты меню «+» в полосе вкладок боковой панели (props `{section, onNavigate, sessionId, placement}`; используйте `DropdownMenuItem`) |
| `shell.canvas.tabs` | Keyed: вкладка боковой панели (справа от чата, снизу в режиме разработки). Компонент вызывается с `part:'tab'` для заголовка вкладки и `part:'body'` для содержимого; props `{section, onNavigate, sessionId, placement, part, selected}`. Открыть вкладку: `canvas.open(entryId)` из `apps/web/src/app/canvas` |
| `shell.titlebar.menu` | Пункты меню «⋯» в шапке над свитчем «Не давать уснуть» (props `{section, onNavigate, sessionId}`; используйте `DropdownMenuItem` из `@minerva/ui`) |
| `shell.titlebar.start` / `shell.titlebar.end` | Шапка окна: после кнопки меню / перед кнопками окна (индикаторы сервисов) |
| `shell.content.before` / `shell.content.after` | До / после основного содержимого |

В режиме разработки list-слоты `editor.tabs.start` и `editor.tabs.end`
размещают компоненты соответственно до и после вкладок редактора. Они доступны
в workspace scope и получают `{sessionId, activePath}`, где `activePath` равен
`null`, если файл не выбран. Регистрации удаляются при отключении плагина.

Встроенные разделы: `chat`, `plugins`, `settings`. Вызов `onNavigate` переключает
раздел и закрывает мобильную Sheet. При отключении активного plugin-раздела
открываются чаты. Чат остаётся смонтированным при переходах, его черновик
и выполняющийся запрос сохраняются. Существующие `navigation.items`,
`settings.pages`, conversation/composer/message/tool слоты продолжают работать.
Настройки теперь страница: `settings.pages` по-прежнему получает
`placement: 'navigation' | 'body'`, `selectedPage` и `onSelectPage`.

Ширина панели ограничена 192–480 px и доступным местом для основной области;
мышь и клавиатура обслуживаются `Resizable`. Двойной щелчок по разделителю
возвращает 272 px. Ширина и состояние сворачивания сохраняются локально.
На экране уже 640 px боковая панель открывается как Sheet с Escape и возвратом фокуса.

## Регистрация контекстных меню

Как в DSH, действия регистрируются отдельно от React-представления через
`ctx.contextMenus.register({id, target, order, resolve})`. Регистрация принадлежит
плагину и текущему scope; её не нужно вручную удалять в cleanup. ID регистрации
уникален внутри scope. Сортировка: `order`, затем ID.

`resolve(value, signal)` получает захваченную цель и возвращает массив описаний
действий (или Promise). Действие содержит `id`, `label`, `run()`, опциональные
`disabled`, `danger`, `separatorBefore` и `icon: {key, src?}`. Возврат `[]`
скрывает действия. Подготовка ограничена одной секундой; закрытие меню и выгрузка
плагина отменяют её через AbortSignal. Ошибка одного провайдера показывается в
меню, не блокируя остальные. Вызовы пунктов выгруженного плагина отклоняются.

```ts
ctx.contextMenus.register({
  id: 'my-plugin.inspect-workspace', target: 'workspace', order: 200,
  resolve: ({workspace, disabled}, signal) => [{
    id: 'inspect', label: 'Открыть сведения', disabled,
    run: () => inspectWorkspace(workspace.id),
  }],
});
```

Внешние плагины уже получают scope приложения workspace; системный плагин
выбирает его через `ctx.scope(workspaceScope)`. Scope приложения не является
ID папки. Цель workspace имеет вид
`{kind:'workspace', value:{workspace:WorkspaceInfo, disabled:boolean}}`.
Новые семейства целей расширяют интерфейс `ContextMenuTargetMap` в SDK.

Для диалогов цель имеет вид `{kind:'session', value:{session:SessionInfo,
workspaceId:string, pinned:boolean, archived:boolean, disabled:boolean}}`.
Встроенные регистрации: `session.pin` (100), `session.rename` (200),
`session.fork` (300), `session.archive` (400), `session.copy` (500).
Плагин может добавить свои действия в тот же реестр. Подготовка ветки проверяет
текущий snapshot: при выполняющейся задаче пункт недоступен; ядро повторяет эту
проверку в транзакции перед копированием.

Иконки — отдельный keyed-слот `contextMenu.icon` в workspace scope.
`entryId` совпадает с `icon.key`, компонент получает `{icon}`. Описания действий
не содержат JSX. Общий `ContextMenu` экспортирован из `@minerva/ui`; он принимает
`target`, `scope`, `label` и `children`, открывается через `…`, правую кнопку,
Shift+F10 или ContextMenu, сохраняет цель до закрытия и возвращает фокус.

Встроенные rename/remove использует сам `workspaces-ui`: порядки 100 и 300,
иконки `workspace.rename`/`workspace.remove`. Обработчики передают захваченную
цель диалогам подтверждения, независимо от выбора текущего чата.

Прежний `workspace.menu` оставлен как адаптер для существующих React-плагинов:
его пункты идут после действий реестра. Новые действия используют реестр.
Подменю и нативные OS-меню в этой реализации не добавлены.

## Исходный подход

- [DSH module loader](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/client/modules/src/client/system.ts)
- [DSH shared modules](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/client/web/src/platform.ts)
- [DSH settings rendering](https://github.com/deepseek-ai/deepseek-harness/blob/fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159/packages/client/ui-settings-general/src/client/SettingsRoot.tsx)

DSH распространяется по MIT, copyright 2026 DeepSeek. В этой реализации его исходный код не копируется: сохраняются архитектурные принципы и используется существующий Minerva plugin SDK.

## Боковая панель (canvas)

Справа от чата (в режиме разработки — под редактором) находится панель с вкладками.
Постоянную вкладку плагин регистрирует в keyed-слоте `shell.canvas.tabs`. Вкладку с
произвольным содержимым — разметкой, виджетом, фреймом — можно открыть в любой момент:

```tsx
const {canvas} = require('@minerva/ui');
canvas.show({id: 'my-plugin:report', title: 'Отчёт', icon: <ChartIcon/>,
  render: ({sessionId, placement}) => <Report sessionId={sessionId}/>});
canvas.closeTab('my-plugin:report');
```

`show` с тем же `id` заменяет содержимое вкладки и делает её активной. Такие вкладки
закрываются крестиком и не переживают перезагрузку окна. Действия, открывающие вкладки,
удобно класть в меню «+» через слот `shell.canvas.actions`.
