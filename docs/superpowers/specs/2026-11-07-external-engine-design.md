# External engine package

Current execution extensions and limits: [backend plugin contract](../../backend-plugins.md). The complete-harness spec supersedes the earlier no-tools/no-approval restriction below.

UI revision: the original HTML/iframe design below is superseded by [native plugin UI](../plans/2026-11-07-native-plugin-ui.md) and [SDK v2 contract](../../plugin-ui.md). `plugins.ui` now returns a verified CommonJS React module; the engine/backend contract is unchanged.

User authorizes restoring the former Codex functionality as an external package, without legacy history migration. Source lives in `D:/develop/minerva-codex-plugin`, outside the core workspace. Core must build when that directory is absent. The first external package supports Windows only; Unix execution fails closed until supervision can guarantee descendant cleanup after abrupt plugin death.

## Contract

Installed local trusted process packages may declare `model-engine` and `local-environment` capabilities. The latter receives only PATH/HOME/USERPROFILE/APPDATA/LOCALAPPDATA/SYSTEMROOT/TEMP/TMP. The plugin uses USERPROFILE/HOME for default Codex home; vendor-specific variables and API-key environment are not forwarded. Hosted native packages remain disabled. Initialize receives `apiVersion`, `pluginId`, and an absolute mutable `dataDir` outside the immutable package.

Engine RPC is bounded request/response polling; long model calls run inside the plugin, never while the host waits on a single RPC. All engine methods are reserved to the host, inaccessible through browser `plugins.call`.

| Method | Params | Result |
| --- | --- | --- |
| engine.models | {} | {models:[{id,name,isDefault}],loading:boolean,error:string|null} |
| engine.start | {runId,model,turns:[{role,parts,replay}],state:JSON|null} | null |
| engine.poll | {runId} | {status:'running'/'completed'/'failed'/'cancelled',transcript:{parts,replay},state:JSON|null,error:string|null} |
| engine.cancel | {runId} | null |
| engine.release | {runId} | null |

Host records checkpoints through the existing Transcript validation/persistence path and atomically commits opaque engine state with the final answer. State is namespaced by plugin ID and anchored to model and parent response. Before a new run the previous state is invalidated; failure/cancellation cannot reuse it. Drop/shutdown aborts the plugin process tree. Polling is 200ms, overall deadline 240s. Plugin bounds concurrent jobs, releases terminal jobs, aborts task on cancel.

Core endpoints: `plugins.models` returns `PluginModel[]` ({id,name,group,isDefault}, IDs `plugin:<package-id>/<model-id>`); `plugins.ui` ({id}) returns self-contained HTML for an enabled installed package with UI. PluginInfo adds hasUi. Core never contains Codex model/auth/settings types.

UI is an opaque-origin sandboxed iframe (`allow-scripts` only) with a CSP disallowing network/resource loads. Bridge postMessage validates iframe source and request ID; forwards only scoped `plugins.call` with fixed plugin ID and blocks engine methods. Replies use `{type:'minerva.result',id,result}` or `{type:'minerva.result',id,error}`. Requests use `{type:'minerva.call',id,method,params}`. Frame UI methods are short: `settings.get` returns {preferences:{hiddenModels:[],defaultModel:'',options:{effort:null,summary:'auto',verbosity:null}},models:CodexModel[],status:CodexStatus|null,login:LoginStatus|null,loading:boolean,error:string|null}; `settings.save` takes preferences, validates and atomically saves; `account.refresh` and `account.login` enqueue background work and return null. UI polls settings.get, starts refresh after login completes. All external text is rendered as text, not HTML.

User can enable installed package, open settings, log in, view account/quota, hide models, choose default and response options; models appear in normal chat selection. Existing local Codex CLI auth is reused. Prior read-only sandbox and no-approval behavior are retained. No filesystem tools or approval features are added. CLI availability and live account/catalog are checked; paid model test uses a tiny prompt only if local authorization is available.

## Implementation tasks

1. Root: move neutral transcript/turn wire types to Rust SDK; extend host capability/environment/data/UI support; add neutral engine orchestration, endpoints and tests. Keep existing host test compatibility.
2. Backend implementer: standalone Codex package, restored protocol/history code, bounded polling server, preferences, package build script, tests and README. No edits to core crates.
3. UI implementer: generic PluginPanel bridge, model selector integration and client endpoints; external self-contained Codex settings HTML. No edits to Rust.
4. Root: integrate, inspect diffs, test both workspaces, package and install in a disposable host; real CLI smoke; final review and documentation. Do not overwrite running user binaries or delete user databases.

Use subagent-driven-development for the independent backend/UI work, then review combined contracts and lifecycle. No commit/push is requested. Existing user changes remain untouched.
