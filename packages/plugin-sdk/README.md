# Browser plugin SDK (C2 foundation)

The SDK entry is React-free. It exports scoped slot and typed service registries,
plugin contexts, disposable ownership, and a serialized BrowserPluginHost.
React rendering is isolated behind the `/renderer` export, with a per-plugin
error boundary and settings/disable recovery actions supplied by the host.

Only trusted, already-resolved BrowserPlugin objects are accepted. This package
does not fetch modules, evaluate code, authorize tenants, or assert backend health.
UI props must contain presentation data and scoped actions; never pass lifecycle
contexts, transport instances, secrets, or service implementations to components.

Slot selection is a profile map from slot name to plugin ID (single), or from
entry ID to plugin ID (keyed). Unselected conflicting providers are rejected.
Disposing a selected replacement restores its previous registered implementation.
Lists sort by order, plugin ID, then entry ID, independent of activation order.
Scopes are exact: session registrations include both workspace and session IDs.

A context owns its registrations, listeners, cleanup functions and child contexts.
Disable rejects when active dependent plugins remain. Activation validates the
whole dependency graph before applying anything, applies dependencies first,
and rolls back newly applied plugins if any application fails. Existing plugins
remain active. Cleanup continues through errors and reports an AggregateError.

Cordis is pinned to **4.0.0-rc.12** (MIT), inspected through npm primary metadata
and installed public declarations: `Context`, `Context.plugin`, fiber readiness,
`Context.effect`, and fiber disposal. Source: https://github.com/cordiverse/cordis
(package `packages/core`). This is a prerelease; future upgrades require lifecycle
regression tests. There is no implied wire compatibility with Rust Cordis or DSH.

Run `node --test tests/plugin-sdk.test.mjs` from the repository root (Node 24+).
