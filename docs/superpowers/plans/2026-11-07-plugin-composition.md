# Backend plugin composition and master switch

User approved completing backend composition and adding a manager toggle for non-system plugins. Continue in the existing working tree; preserve previous uncommitted work and do not commit.

## Design and contract

- System policy is host-owned: plugin-manager, providers and shell cannot be disabled. External manifests cannot grant system status.
- Master switch pauses optional plugins without overwriting their individual enabled intent. Persist it in the existing plugin lock with a backward-compatible default true. Resume only selected plugins; optional builtin example selection is persisted too.
- Keep list_plugins returning PluginInfo[]; add optionalEnabled boolean (same global value on every row) and requestedEnabled boolean. Add set_optional_plugins_enabled({enabled}) returning the roster, through HTTP and Tauri.
- requires remains named versioned service requirements (not package IDs). Add provides named versioned services. Validate missing/ambiguous/version-mismatched services and cycles, activate dependencies first and stop dependents first. Failed dependencies block consumers and remove their UI. Reject unsupported host service contracts explicitly; no fictional sessions/tools access.
- Separate package/install/lock, process transport and dependency composition responsibilities within plugin-host. Preserve process safety, digest checks and independent unrelated RPC.
- Provide actual declared service routing for process consumers, not only startup ordering; validate declared access and bound requests. All native plugins remain trusted local code.

## Tasks

1. Host/SDK implementer: service graph, lifecycle reconciliation, persisted optional switch and example intent; focused real-process tests. Own crates/plugin-host and crates/plugin-sdk/src/lib.rs.
2. Root: runtime system policy and optional builtin lifecycle; roster DTO and HTTP/Tauri/client wiring; tests, docs and integration.
3. UI implementer: manager-row accessible switch and paused state, model race fencing, JS/browser tests. Own client-models/plugins.ts and Plugins.tsx and own new tests.
4. Review and verify: host/runtime/API tests, frontend tests/build/contracts, Clippy, browser real Host master pause/resume/restart, desktop Codex smoke. Preserve user data; no paid model calls.

## Rulings

- Reuse existing list contract with additive fields rather than a second polling endpoint. The toggle controls optional execution, not installation or permissions.
- Startup selection survives pause and restart. Explicit per-plugin changes while paused change selection only and do not execute plugins.
- No automatic rerun of interrupted chat or tools on resume.

## Completed and verified

All four tasks completed in the existing working tree. Host now separates package, storage, process and graph modules. Real service routing uses services.call from a consumer and services.invoke on the provider, preserving namespace and declared version. The manager switch is exposed through HTTP, gateway policy, Tauri and client models.

Review caught successful stale replies after disable and cancellation midway through stopping multiple processes. Both now have real-process regression tests; Host rejects superseded replies and owns cleanup for all fenced processes. Runtime accepts lifecycle changes into owned tasks so a disconnected UI cannot abandon the optional builtin mid-transition. Shutdown rejects queued new changes. Narrow re-review found no remaining P1/P2 in these fixes.

Validation: 22 host tests, runtime/API suites and focused manager cancellation/restart tests, gateway registry policy tests, 61 JS tests, five browser tests, TypeScript/Vite build, generated contract check and strict Clippy for host/SDK/runtime/API/gateway. Real Tauri: two optional plugins pause while three system services stay active; requested selection preserved; Codex settings reappear without iframe. No paid model calls; Unix execution not checked.
