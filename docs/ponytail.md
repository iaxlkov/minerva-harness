# Ponytail в Minerva

[Ponytail](https://github.com/DietrichGebert/ponytail) подключён как project-local
правила и skills для агента разработки.

- Версия: **5.3.3**, commit `96608e078b2d5ea607c5c3b8b5a54d1c1fba6846`.
- Источник: [официальная установка](https://github.com/DietrichGebert/ponytail/blob/96608e078b2d5ea607c5c3b8b5a54d1c1fba6846/INSTALL.md).
- Общие правила: [AGENTS.md](../AGENTS.md), исходный текст сохранён.
- Skills: `.agents/skills/ponytail*`; существующие skills не изменены.
- Лицензия: [MIT, Copyright (c) 2026 DietrichGebert](../.agents/PONYTAIL-LICENSE).

В Codex команды появятся на следующем ходу; если IDE ещё не обновила список,
откройте новый чат в этом проекте.

| Команда | Назначение |
|---|---|
| `$ponytail` | Минимальное полное решение; default full, доступны lite/ultra/off |
| `$ponytail-review` | Проверка изменения и связанных вызовов |
| `$ponytail-audit` | Аудит репозитория |
| `$ponytail-debt` | Список отмеченных `shortcut:` |
| `$ponytail-gain` | Опубликованные benchmark-данные автора, не измерение Minerva |
| `$ponytail-help` | Справка upstream; для этой установки используйте имена без `ponytail:` |

Установка выполнена штатным skill-installer с `--dest .agents/skills` и
фиксированным `--ref`. Для обновления сравните изменения upstream и обновите
только эти шесть каталогов, AGENTS.md, лицензию и записанный commit.

Lifecycle hooks и глобальный Codex plugin не установлены. Правила загружаются
из AGENTS.md; выбор уровня действует в контексте чата, автоматического
переключения через hooks и Claude Code band здесь нет.
