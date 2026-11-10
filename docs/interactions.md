# Вопросы и разрешения

Встроенный browser-плагин `plugins/mh-interactions/ui` подключает селектор доступа
в `composer.access`, панель контекста и вопросов в `session.controls`, а подтверждения
конкретных вызовов — в `tool.interactions`. Композитор и сообщения остаются
assistant-ui. Отключение UI-плагина не выдаёт разрешений.

Второго протокола вопросов нет: используется существующий серверный
`InteractionBroker` и `interactions.list/answer`. Ответ относится к точным
session/run/call и ID запроса; подтверждение продолжает ожидающий вызов, не отправляя
его повторно. Отмена или истечение запроса делают поздний ответ недействительным.

Селектор сохраняет доступ только текущей сессии. «Полный доступ» разрешает побочные
эффекты без запроса; в остальных режимах вызов с побочным эффектом ждёт разового
подтверждения. «Только чтение» допускает чтение без подтверждения, изменения требуют
исключения на один вызов. Политика не отменяет проверок путей и схем инструментов.
В настройках сессии остаются параметры контекста; их сохранение сохраняет актуальную
политику доступа.

## Сопоставление с DeepSeek Harness

Проверены исходники upstream на 2026-11-10:

- [user-questions](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/interaction/user-questions/src/index.ts): нейтральный сервис вопросов и scoped answerers.
- [user-approval](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/interaction/user-approval/src/index.ts): серверный сервис одноразовых решений, отмены и политики.
- [ui-approval](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-approval/src/client/index.ts): отдельный browser-плагин, связывающий ожидающее решение с UI через слоты.
- [ui-permission-presets](https://github.com/deepseek-ai/deepseek-harness/blob/master/packages/client/ui-permission-presets/src/client/index.ts): отдельный селектор текущей сессии в композиторе.

Используется это разделение ответственности, а не прямое подключение npm-пакетов
DSH: их Agent, Remote waterfall, Session projections и Cordis API отличаются от
Rust-хоста Minerva. Существующий брокер Minerva остаётся источником ожидающих
запросов. Не переносились timed questions, поздние ответы отдельным ходом,
постоянные правила и смена sandbox-профилей.
