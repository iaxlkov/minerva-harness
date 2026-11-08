# Workspaces

Workspaces are implemented by the mandatory `workspaces` Cordis system plugin
(`studio.workspaces`), with `studio.sessions` and `studio.context` as dependencies.
The `workspaces-ui` browser plugin contributes the grouped sidebar through
`navigation.items`. Disabling optional plugins does not disable the registry.
The authenticated host workspace scope remains separate from these folder projects.

The registry and membership tables live alongside session context in
`context.sqlite3`. Existing session histories and settings are retained. Sessions
with an existing cwd are grouped by canonical path; other sessions join Default
workspace. Its directory is `<runtime data directory>/default-workspace` in local
and desktop profiles. Hosted profiles cannot register local folders.

`workspaces.list` returns folders with their session IDs. `workspaces.create`
validates an existing absolute directory and reuses a registered canonical path.
`session_create` accepts an optional `workspaceId`, defaulting to `default`.
Context and tools resolve cwd from the immutable membership. Session settings
still own permission policy and context limits, but cannot change the folder.
Missing folders keep their history and prevent creation of new sessions there.

The row menu opens through `…` or right-click. `workspaces.rename` changes only
the display title (1–128 characters); `workspaces.remove` hides a non-default
workspace after UI confirmation. The `hidden_projects` table preserves the folder,
session membership, history and running tasks. Opening the folder again restores
the same workspace and title. Startup selects only visible workspaces. The default
workspace can be renamed but cannot be removed. Built-in menu actions and plugin
contributions use `ctx.contextMenus.register` with target `workspace`; the old
`workspace.menu` slot remains a compatibility adapter (see `plugin-ui.md`).

Desktop uses Tauri's dialog plugin with only `dialog:allow-open` on the main local
window. Web uses an explicit host-side path form. Cancelling the desktop chooser
does not create a workspace or session. No user files are moved or deleted.

References reviewed:

- [DeepSeek workspace registry](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/workspace/workspace)
- [DeepSeek workspace UI](https://github.com/deepseek-ai/deepseek-harness/tree/master/packages/client/ui-workspace)
- [Rig agents](https://www.rig.rs/docs/concepts/agent): agent execution and tools remain separate from project navigation.
- [Tauri dialog](https://v2.tauri.app/plugin/dialog/)

Validation: workspace storage/migration tests, system plugin lifecycle and namespace
checks, HTTP RPC tests, browser session-settings/layout tests, and the real Host
workspace flow in `tests/e2e/workspaces.spec.mjs`.
