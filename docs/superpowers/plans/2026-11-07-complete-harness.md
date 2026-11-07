# Complete agent harness implementation plan

> Execute with superpowers:subagent-driven-development. The user explicitly authorized subagents, implementation, tests, commits and push.

**Goal:** Deliver all eight baseline capabilities and both state-rendering fixes from the audit.
**Architecture:** DSH-style scoped services and effect-owned registrations adapted to Rust Arc/cancellation, Rig's existing loop, durable SQLite transcript and React slots. Core stays provider-neutral; Codex stays external.
**Spec:** `docs/superpowers/specs/2026-11-07-complete-harness-design.md`.

## Constraints and ownership

Existing uncommitted platform work is a prerequisite and must be retained. Work in `feat/web-plugin-platform`; no worktree copying or resetting the user's tree. Root alone edits shared integration files (`runtime/{lib,plugins,providers,conversation,run_supervisor,sessions,session_store,rig_history}.rs`, API, Tauri, contracts, Cargo manifests, client endpoint declarations). Agents own separate modules and tests. No agent commits; root stages reviewed task files explicitly. Skills/assets/bun.lock are excluded unless a verified runtime reference requires them.

## Tasks

- [x] **T1 — tool/interaction services.** Agent owns `crates/runtime/src/tools/**`, `crates/plugin-sdk/src/tools.rs`, focused tests. Read DSH contracts/pipeline/native executor/approval/questions source. Implement registry, revocable owner registrations, validated schema, per-step snapshot, timeout/cancel, exact identity approval/question broker, native workspace read/search/write/patch/shell handlers, and Rig dynamic tool adapter. Tests first for escaped paths, deny/ask/allow, stale answers, cancellation and side effects. Send root concrete service API and dependency requests. Root integrates module/Cargo and plugin registration.
- [x] **T2 — workspace/context/skills/compaction/attachments.** Agent owns `crates/runtime/src/context/**`, `crates/plugin-sdk/src/context.rs`, focused tests. Read matching DSH sources. Implement atomic session-scoped settings, project instruction chain, skill discovery/load, budget/pruning and anchored non-destructive summary projection, MIME-validated durable raster image store with owner checks. Expose APIs from spec and a prepared context for root's Rig runner. Tests prove actual files, prefix anchoring, reload and cross-session denial. Root supplies model-based summarizer and actual wire routing.
- [x] **T3 — execution presentation.** Agent owns `apps/web/src/{ToolHistory,sessionHistory,transcript}.ts*`, new timeline UI module/components, structured history and new browser fixtures. Read DSH ui-tool/assistant-stream. Fix pending-vs-interrupted and running-vs-incomplete before richer tool cards; preserve unknown fallback, call/result IDs, reasoning, interrupted prefixes and reload. Do not edit Chat/App shared integration yet. Expose image renderer and interaction/settings panel components in new module for root to mount.
- [x] **T4 — core run integration and API stream.** Root integrates T1/T2; uses current Rig DynamicTool/ToolSet and streaming hooks rather than writing a second loop. Live text/reasoning/tool execution snapshots retain durable native replay and per-call state. Prepare workspace prompt/skills before model steps, inject admitted plugin/native tools, record usage and output budget, compact through selected model, pass images as native content. All routes remain cancellation-owned and hosted constraints explicit. Add mock provider wire tests for tools, streams, continuation, compaction and images.
- [x] **T5 — MCP services.** Fresh agent owns `crates/runtime/src/mcp/**`, SDK DTO file and fixture tests. Starts after T1 contract is fixed. Read DSH MCP connection/generation/deadline sources. Implement stdio and HTTP lifecycle, init/list/call, saved scoped settings, bounded transport/recovery and registration into T1. Do not replay tool effects. Root adds API/Cargo/manager ownership.
- [x] **T6 — client feature integration.** Fresh agent owns settings/interaction/attachment/MCP panels and client models after endpoint contracts generated; root coordinates Chat/settings imports. Add workspace picker/settings, pending approval/questions, attachments and previews, MCP management, context budget/compact action. Browser tests must inspect actual backend behavior, not only mocks. Preserve plugin slots and master pause/resume.
- [x] **T7 — external Codex parity and engine context.** Fresh agent owns neighboring `../minerva-codex-plugin` changes when core context contract is stable. Restore icon/tabs/reasoning options; accept neutral prepared context/images/workspace settings without pulling implementation back into core. Test against codex-fixture only, package and reinstall local Tauri package preserving `.data`. Root coordinates independent repository commit/push only if its remote exists; otherwise explicit packaged artifact with tracked source reference.
- [x] **T8 — review, complete verification, commit and push.** Fresh reviewer inspects all requirement boundaries and risky data writes. Root fixes findings, verifies Rust fmt/Clippy/workspace tests, JS/types/build/contracts/browser suite, and native Tauri disposable fixture. Inspect git diff/status for secrets/unrelated assets, commit reviewed implementation, `git push origin HEAD`, then verify remote commit equals local. Do not mark goal complete until spec evidence checklist is satisfied.

## Integration overlap audit

| Tasks | Shared interface | Owner/rule |
|---|---|---|
| T1/T4/T5 | Tool registry/execution context | T1 publishes concrete API; T4/T5 consume it |
| T2/T4/T6 | WorkspaceSettings/context/image wire DTO | T2 SDK DTO; root generates types; UI consumes |
| T1/T3/T4 | Transcript tool status/meta | Additive fields in existing ContentPart.data; root owns SDK transcript edits |
| T1/T6 | PendingInteraction and answer identity | SDK tools DTO; root endpoints; UI polling/answers |
| T4/T7 | engine.start context/attachments | Neutral optional params; external plugin must explicitly handle support |
| All | Cargo/shared API/git | Root owns mutations and commits; agents request changes |

## Execution ledger

Per-task progress and implementation decisions live in `.superpowers/sdd/2026-11-07-complete-harness/progress.md`. Tests must first fail for the intended missing behavior, then pass. Task reviews cover spec and code quality; final review covers whole integration. No completion by narrowing scope or by merely adding unused APIs.


## Verification (Windows, 2026-11-07)

- `cargo test -j1 --workspace -- --test-threads=1`:182 passed,5 explicitly ignored (four disposable PostgreSQL checks and the opt-in user-history migration check).
- `cargo clippy -j1 --workspace --all-targets --no-deps -- -D warnings` and `cargo fmt --all -- --check`: passed.
- `npm test`:72 passed. Full Playwright suite:17 passed; targeted MCP browser check repeated after the final CSS spacing fix and passed.
- `npm run build`, generated contracts check, server build and native desktop binary build: passed.
- Actual Tauri IPC + visual inspection: external Codex icon/tabs/model-supported reasoning options, cached draft, workspace/context, validated image roundtrip, MCP stdio/HTTP discovery and cleanup. Normal app relaunched; temporary CDP listener closed.
- External Codex repo `kviron/minerva-codex-plugin` is private, independent of the core build, and pushed at8031dbf. Root repeated13 Rust tests and5 controller tests; actual core-host fixture proved install/enable/catalog/UI, two-turn resume and cancellation without model charges.
- Native/plugin/MCP tool execution, streaming reasoning/text, allow/deny/cancel/reload, exact questions, immutable compaction and real image wire content were exercised against actual Rust hosts and deterministic provider/process fixtures.

Limits: no paid model requests, no Unix execution, no disposable PostgreSQL or user-database migration run. One Windows keyring integration test returned a transient missing-key error in a parallel attempt; it passed individually, in a full parallel runtime repeat and in the final sequential workspace run. The Vite build still reports a roughly1MiB minified main bundle; code splitting is outside this baseline scope.
