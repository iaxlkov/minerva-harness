# Plugin isolation fixes

Scope: four audited failures, preserving the external process protocol and native UI SDK.

1. Host discovers invalid packages as failed entries rather than aborting app startup. Restore continues after individual activation failures. Roster exposes per-plugin error and actual active status; requested enable state stays in the lock. Invalid/missing entries can be disabled, and retries revalidate packages. Global unreadable configuration/storage errors still fail explicitly.
2. Each plugin serializes its own process calls. No registry-wide lock is held while awaiting RPC, initialization or shutdown. Disable fences queued work and owns teardown; cancellation of a call must not poison framing for later calls. Retain digest checks, process-tree cleanup, bounded frames/timeouts and writer lock merging.
3. Runtime models validates each catalogue independently. Invalid entries add a per-plugin diagnostic and do not discard healthy catalogues. Successful subsequent catalogue clears that diagnostic.
4. Frontend tracks backend roster continuously with bounded non-overlapping refresh, displays errors and synchronizes external UI activation/removal after a crash. Refreshes must not overwrite newer toggles or survive disposal.

Work ownership: Host implementer owns crates/plugin-host plus its tests/fixture; UI implementer owns frontend polling/status and TS model tests; root owns runtime DTO/manager wiring, contracts generation, catalogue isolation tests, integration, review and verification.

Contract: PluginInfo adds `error: string | null`; existing `enabled` denotes actual active state. No new lifecycle enum in this patch: synchronous toggles already display pending UI; failed vs disabled is distinguished by error. This avoids advertising states the backend does not yet reliably emit.

Tests: broken disabled package does not prevent boot; one startup failure does not suppress healthy plugins; slow RPC does not delay unrelated RPC/roster; cancellation/disable races cannot reuse old process; bad catalogue preserves healthy models; crash removes UI and failed status is visible; toggle-refresh races fenced. Run affected Rust/JS checks, browser tests, formatter/Clippy, and desktop smoke with installed Codex. Icons/tabs remain a separate pending correction.

Completed: per-process Host locking, failed-entry diagnostics/retry, generated error DTO, independent parallel catalogue validation, and two-second non-overlapping roster polling with UI teardown. Review additionally caught redundant enable invalidating an active RPC; fixed with an idempotent enable and a same-process/generation regression test.

Validation: 15 plugin-host tests; 43 runtime/API tests passed (one pre-existing opt-in migration test ignored); 59 JS tests; four browser scenarios including real process isolation; TypeScript/Vite build; generated-contract check; strict Clippy for host/runtime/API. Desktop smoke: installed Codex active with error=null, six models, native settings visible, zero iframes. No paid model requests or Unix execution. Initial Vite dependency reoptimization needed a page reload; temporary WebView debug port removed after smoke.
