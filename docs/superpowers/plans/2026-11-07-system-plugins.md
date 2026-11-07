# Built-in system plugins

**Goal:** Make the existing backend capabilities real, mandatory Cordis plugins compiled into the application. The user approved this boundary; Codex remains external.

**Reference:** DSH commit `fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159`, `packages/mcp/mcp-manager/src/index.ts`, `packages/mcp/mcp-client/src/index.ts`, and manager profile composition tests. MCP manager publication and the lifetime of its connections/tools belong to its plugin.

**Design:** Keep the existing Rust services and database paths. Resource construction may happen before activation, but publication, native tool registration, MCP startup and cleanup belong to Cordis effects. The root keeps session/run storage and execution policy. Add mandatory plugins for the tool runtime, context (workspace, instructions/skills, summaries and attachments), native workspace tools and MCP alongside the existing manager/providers/shell. Keep a single runtime catalogue for roster and mandatory-toggle checks; reserve all IDs in the package SDK.

The optional-plugin switch affects installed packages and the example plugin. System MCP stays active; individual configured servers retain their own enabled setting. No new dependency, database migration, external package conversion, or change to the concurrent frontend layout work.

- [x] Add failing regressions for system roster/protection, actual Cordis service publication and owned tool cleanup, MCP surviving optional pause, reserved IDs.
- [x] Move activation into `crates/runtime/src/plugins/builtin.rs`; remove active wiring from `harness.rs` and manual MCP startup/shutdown from the manager. Dispose all started fibers in reverse order, including failed startup.
- [x] Update affected lifecycle/manifest expectations and document the system/optional boundary.
- [x] Run focused Rust tests, runtime/API regressions, Clippy/fmt and frontend compatibility checks. Stop any owned dev watcher before edits; restore a normal desktop launch after verification without leaving an orphan Vite.

Public HTTP/Tauri DTOs, existing model/tool contracts, settings and credential locations remain unchanged. Context services remain grouped because they share one SQLite projection/store; do not invent four wrappers with no independent responsibility.


## Verification

- Missing-system-plugin roster regression observed failing before implementation.
- Runtime unit/integration tests:102 passed,1 existing opt-in migration ignored. Includes real MCP discovery/call while optional plugins are paused and restart with the pause persisted, exact Cordis service publication/withdrawal, failed activation rollback and native/MCP admission revocation.
- HTTP/worker API tests:9 passed. Reserved builtin ID regression passed for every system ID.
- Workspace Clippy all-targets with `-D warnings`, Rust formatting and scoped diff checks passed; JavaScript suite72 passed.
- Standard `npm run tauri:dev` completed the desktop build and opened Minerva Harness. No frontend layout edits, new dependencies, database changes or paid model calls were required.
