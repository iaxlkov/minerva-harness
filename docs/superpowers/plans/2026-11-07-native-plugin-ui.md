# External React UI modules

User requests DSH-like plugin UI architecture, replacing the temporary iframe path. Trusted installed code may execute in the frontend context. Keep vendor source outside the core workspace.

## Contract and implementation

1. UI manifest `sdkVersion: 2`, entry `ui/client.js`: a bundled CommonJS module exporting `sdkVersion = 2` and `apply(ctx)`. No HTML fallback. Host verifies package digest and serves source via existing `plugins.ui`; incompatible SDK fails explicitly.
2. Browser loader shares `react`, `react/jsx-runtime`, `react-dom`, `react-dom/client`, and `@minerva/ui` (existing Button and Select primitives). It caches module evaluation for its enabled lifetime, activates through BrowserPluginHost, and serializes enable/disable reconciliation. In-flight loads cannot activate after disable/disposal. Errors must be visible and recoverable.
3. External apply receives a workspace-scoped BrowserPluginContext plus `rpc(method, params)` bound to its own backend. Host-only methods remain blocked. Existing slots, ownership and error boundaries remain authoritative. Disabling disposes controllers/slots/listeners. Closing a settings panel does not dispose the plugin.
4. Settings uses only the existing settings.pages slot contract (navigation/body). Remove metadata-generated duplicate UI and iframe bridge. Selection falls back to Models when its slot disappears.
5. External Codex UI becomes React using the shared UI library. Controllers and drafts are created in apply, own cached state and subscriptions, and survive remount. Poll only with mounted subscribers; cancel timers/fence requests on disposal. Restore existing login, quotas, model visibility/default and response options.
6. Package, install/update user's Windows desktop plugin through verified installer, and test real Tauri. Preserve preferences. No history migration.

## Verification

Loader lifecycle tests: shared React identity; one evaluation; enable/disable races; failed apply cleanup; reserved RPC denial; cleanup on scope disposal. Browser test: settings slot appears, native DOM (no iframe), state/draft preserved across close and section changes; disabled plugin vanishes. Build, TS tests and affected Rust tests; actual installed package settings and model catalog.

## References and scope

DSH reference checkout: agent-artifacts/reference-dsh, commit fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159, MIT (2026 DeepSeek). Follow its shared module table + cached factories + slot registrations + external stores. Preserve license attribution for copied substantial code. Our Rust RPC source transport differs from DSH's HTTP bundle transport. Cross-package imports, multi-chunk bundles and live HMR updates of installed packages are out of this change; disable/update/re-enable refreshes module source.

## Completed validation

- 54 Node tests; 2 Playwright tests, including production Blob loader under local CSP, shared React hooks, one source load/evaluation, draft retention across close/switch, disposal and re-enable.
- 9 plugin-host lifecycle/manifest tests; 6 local HTTP API tests; TypeScript, frontend build and workspace Clippy pass.
- External package: 5 controller tests, 10 Rust tests, process smoke and real-host fixture smoke pass. Optional live model tests were not rerun for this UI change.
- Installed SDK2 package in Windows desktop, preserving preferences and backing up SDK1 package. Actual Tauri: six models, native DOM with no iframe, unsaved checkbox survives close/reopen, save via scoped RPC succeeds. Restarted ordinary tauri dev after removing diagnostic WebView debugging.
- Separate review found the error-boundary disable path needed to toggle the backend package; corrected it and retained visible disable errors.
