# System plugin packages implementation plan

**Goal:** Separate system plugin implementations into workspace packages while preserving behavior and existing plugin IDs.

**Architecture:** Extract services and Cordis plugins together with their tests into `plugins/mh-*` Cargo packages. Keep runtime-specific composition and adapters in the runtime; move shared contracts only where dependency direction requires it. Keep the current compiled-in distribution and public runtime paths.

**Tech Stack:** Rust workspace, Cordis, existing Node and Rust tests.

## Steps

1. Extract LSP service and Rust language provider as independent packages; retain runtime tool adapter and verify provider lifecycle, existing LSP tests, and runtime integration.
2. Trace service dependencies for workspaces, sessions, agent loop, providers, shell, tools, context, MCP and plugin manager. Extract each implementation with its tests into packages in dependency order; move shared contracts where required without changing persistence or security policies.
3. Update documentation and run workspace, integration, frontend and contract checks. Investigate any failures and document platform-dependent checks.
