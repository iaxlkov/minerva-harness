# Minerva Config and Runtime Services Implementation Plan

> **For agentic workers:** Use executing-plans for integration; independent host configuration work is delegated with dispatching-parallel-agents. Track each deliverable below.

**Goal:** Own sessions and supervised runs through Cordis and use `minerva.conf.json` for composition/settings.

**Architecture:** Keep existing SQLite and conversation executor. System plugin effects own database closure and supervisor admission/cancellation/drain. Plugin host owns a shared validated configuration store protected by its existing writer lock.

**Tech Stack:** Rust, cordis-rs 0.11.1, serde_json, SQLite, existing Tauri/client contracts.

**Spec:** `docs/superpowers/specs/2026-11-07-cordis-composition-design.md`.

## Global Constraints

- Filename `minerva.conf.json`; version 1. Preserve history, credentials and legacy files.
- System plugins remain mandatory; hosted profile cannot gain local capabilities.
- External Codex remains external. Preserve unrelated frontend edits.
- Cargo commands use `-j1` on this Windows machine; stop owned Tauri watcher before Rust changes.

## Task 1: Host configuration and migration

Files: `crates/plugin-host/src/config.rs`, `lib.rs`, `storage.rs`, package/install tests.

- [x] Add regression tests for importing legacy selection/shell, rejecting bad config and preserving digests.
- [x] Observe regression failure, implement `ConfigStore::open`, `snapshot`, `update` with shared writer lock and atomic persist.
- [x] Make host optional/example/individual enabled states read/write this store. Lock file retains package integrity data.
- [x] Verify `cargo test -p minerva-plugin-host -j1` and inspect all failures.

Public integration API: `ConfigStore::open(&Path) -> Result<ConfigStore,String>`; cloneable store; `snapshot() -> Result<CompositionConfig,String>`; `update(FnOnce(&mut CompositionConfig)->Result<(),String>) -> Result<CompositionConfig,String>`. Public `services.shell` and `services.agent_loop.shutdown_timeout_ms` fields.

## Task 2: Actual Cordis ownership

Files: `crates/runtime/src/{lib.rs,run_supervisor.rs,sessions.rs,plugins.rs}`, `plugins/builtin.rs`, `crates/plugin-sdk/src/lib.rs`, runtime lifecycle tests.

- [x] Add failing lifecycle test resolving `SessionService` and `AgentLoopService` from Cordis, checking disposal and stopped admission.
- [x] Publish `sessions` plugin opening/closing storage; publish `agent-loop` plugin owning Runs/tasks/shutdown gate.
- [x] Keep Runtime delegation API and compatibility references, remove its task ownership. Use weak manager reference in agent-loop to avoid permanent ownership cycle.
- [x] Roll back fibers on activation failures and drain runs before closing sessions/plugins. Reserve both IDs in SDK.
- [x] Route legacy HTTP/Tauri session_run through supervised service, preserving transport result shape and durable semantics.
- [x] Verify runtime and API tests; add bounded shutdown/active task regression.

## Task 3: Shell integration, documentation, desktop acceptance

Files: `crates/runtime/src/shell.rs`, docs and config example, required existing tests/fixtures/contracts.

- [x] Test shell save preserves plugin selection and agent-loop config, and reload reads the new authoritative file.
- [x] Integrate shared ConfigStore with existing ShellSettings API; keep old shell.json as migration source.
- [x] Document actual file location, schema/defaults/errors/restart behavior and install workflow. Update service comparison status.
- [x] Run targeted tests, workspace Clippy/fmt, contract checks and JS tests as affected. Check diffs for unrelated files.
- [x] Run actual Tauri: new services in roster, settings persist across restart, MCP visible and real tool call passes approval/durable history.
- [x] Restore normal dev process without CDP. Report limits and uncommitted status explicitly.

## Verification results

- 107 runtime tests and 9 API tests passed; one opt-in database-copy migration test ignored.
- Host suites passed (including 6 configuration regressions); 72 client tests passed.
- Workspace Clippy all-targets with -D warnings, fmt check, contracts check and web build passed.
- Native Tauri UI changed plugin selection and shell limits, restart restored them, then original settings were restored. Legacy lock bytes stayed unchanged.
- Real Codex MCP read passed approval and exact call/result identity; durable transcript survived reload. Reports: agent-artifacts/config-acceptance/report.json and agent-artifacts/mcp-demo/report.json.
- Independent lifecycle/config reviews found no remaining actionable findings.
- Unix, MCP OAuth and hot reload are not covered; manual edits require restart.
