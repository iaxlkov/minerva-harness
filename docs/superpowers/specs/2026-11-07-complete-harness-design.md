# Complete local agent harness

User approved the whole eight-item audit, the two execution-state bugs, subagent execution, tests, commit and push. This is the binding scope, not a reduced milestone. Existing Rust/SQLite/Cordis/Rig/React/Tauri architecture stays. DSH reference commit: `fcc0ad462172522aaf5c93a7b2e01f6b9f0b0159`.

## Required behavior

1. A scoped model-facing tool registry admits an immutable set per model step, validates arguments and output, executes with cancellation/timeouts and records calls/results durably. Native Rust tools and active external plugin tools use the same contract. Disabling a plugin prevents unstarted admitted calls. Rig retains ownership of its model/tools/repeat loop.
2. API text/reasoning streams and tool lifecycle update the existing ordered transcript while running. Completed/reopened history retains the same content. A pending tool is not labelled interrupted, and a running recovered assistant is not incomplete. Tool cards cover generic, file/read/search, diff and shell results; unknown tools have a useful fallback.
3. Sessions select a workspace cwd. Native tools read/search/write/patch files and execute bounded cancellable shell commands within that workspace. Resolve symlink/path escape at boundaries; atomic edits preserve original data on failures. Local tools are disabled for hosted profiles until an execution provider is wired.
4. Execution policy permits reads in the workspace, denies out-of-scope operations, and supports `read-only`, `ask`, `allow` for side effects. Pending approval/questions carry exact session/run/call identity, survive UI reconnect while the run lives, expire/cancel, and reject stale/cross-session answers. Approval is for one call, not a permanent grant.
5. Context assembly combines identity, cwd, applicable AGENTS.md/CLAUDE.md instruction chain, selected skills and tool schemas. Skills have discovery/catalogue plus load-by-name tool; only needed full instructions are loaded. Directory and instruction refresh apply before subsequent model steps. API and plugin engines receive provider-neutral context without core Codex DTOs.
6. MCP manager supports stdio and Streamable HTTP, initialization/tools discovery/call, bounded frames/deadlines, scoped credential-safe configuration, reconnect/retry without replaying tool effects, and lifecycle-owned tool registrations. Nothing is connected by default.
7. Model context/usage budgets are visible. Automatic and explicit compaction summarize older complete turns and preserve recent turns/tool pairing. Original durable history is retained. Summary use is anchored to its exact history prefix and never silently replaces unrelated branches. Large tool results are bounded/pruned for model context without losing their durable diagnostic identity.
8. Durable raster image attachments survive reload, are scoped to their session, displayed in history and actually sent to supported API/plugin model routes. Validate MIME/content/size, reject traversal/cross-session IDs, keep attachment data out of ordinary text logs. Unsupported routes fail explicitly.

Secondary outstanding integration: restore external Codex navigation icon and home/models settings tabs with model-supported reasoning options, keeping the package external. Subagents/plan/jobs/scheduling were explicitly optional in the audit and are not conditions for these eight baseline capabilities; do not implement empty scaffolding for them.

## DSH architectural references

- Tool contracts/pipeline: `packages/core/tools/src/{contracts,pipeline,executor}.ts`; native executor `packages/core/tool-executor-native`.
- Model stream: `packages/core/agent-loop/src/{agent,assistant-stream}.ts`; tool UI `packages/client/ui-tool/src/client/tool`.
- Workspace/file/shell: `packages/workspace`, `packages/fs/{tool-fs,tool-fs-search,fs-patch}`, `packages/shell/{tool-bash,tool-pwsh-persistent}`.
- Approvals/questions: `packages/interaction/{user-approval,user-questions,tool-ask-user}`.
- Prompt/instructions/skills: `packages/core/system-prompt`, `packages/context/agent-instructions`, `packages/skill/{skill,skill-filesystem,tool-skill}`.
- MCP: `packages/mcp/{mcp-client,mcp-manager,mcp-resources}`.
- Context budget/compaction: `packages/compaction/{compaction,compaction-basic,compaction-tool-result-pruner}`.
- Attachments: `packages/attachment/{attachment,attachment-local}`, `packages/client/{file-upload,ui-attachment}`.

Read the relevant implementation for each task, record what was reused conceptually and Rust-specific differences. Do not copy their entire framework or use package presence as proof a feature works.

## Integration contracts

Existing `ContentPart` remains the transcript envelope. Tool-call data extends `{callId,name,arguments}` with optional `status` (`pending`, `running`, `requires-approval`, `completed`, `failed`, `cancelled`) and `meta`. Tool-result data extends `{callId,output,isError}` with optional metadata. Unknown optional fields remain backward compatible. Native Rig replay is preserved separately from display metadata.

Workspace settings wire shape: `{cwd:string|null, permissionPolicy:'read-only'|'ask'|'allow', contextWindow:number, autoCompact:boolean}`. Defaults: no cwd, read-only, 32768 tokens, autoCompact true. A model can still chat without selecting a workspace; filesystem/shell tools require a configured directory.

Interactions wire shape: `{id,sessionId,runId,callId,kind:'approval'|'question',title,prompt,options:string[],createdAt:number}`. Answer input `{sessionId,id,decision:'allow'|'deny'|'answer',answer?:string}`. Decisions are validated against pending identity and accepted once.

Public endpoints to implement in HTTP/Tauri/client/gateway:

```
workspace.get({sessionId}) -> WorkspaceSettings
workspace.save({sessionId,settings}) -> WorkspaceSettings
tools.list({sessionId}) -> ToolDefinition[]
interactions.list({sessionId}) -> PendingInteraction[]
interactions.answer({sessionId,id,decision,answer?}) -> boolean
context.compact({sessionId}) -> ContextSummary
attachments.put({sessionId,name,mimeType,data}) -> AttachmentRef
attachments.get({sessionId,id}) -> AttachmentData
mcp.list({}) -> McpServerView[]
mcp.save({server}) -> McpServerView[]
mcp.delete({id}) -> McpServerView[]
```

AttachmentRef wire shape `{id,name,mimeType,bytes}`; AttachmentData adds base64 `data`. New user submissions carry attachment refs in options.attachments; backend resolves and persists neutral image parts before accepting the run. Never trust client-supplied attachment bytes/metadata without resolving server storage.

Root owns integration APIs and generated contracts. Implementers expose focused Rust services; concrete internal method signatures are recorded in the plan ledger as soon as they compile, then treated as fixed between tasks.

## Evidence required for completion

- Local deterministic model server streams reasoning/text, asks native/plugin/MCP tools, requests write approval, accepts/rejects/cancels, sees tool results and continues. UI must display the same content live, on reload and after interruption.
- Workspace read/search/patch/shell operate in disposable projects; traversal/symlink escape rejected; write denial leaves files unchanged; cancellation stops process trees.
- Skill/instruction changes alter next model context. Skills are loaded on demand. Context compaction reduces model history while original persisted history remains intact.
- Image upload/reload/cross-session access tests and inspection of actual model wire payload containing the image.
- MCP real fixture subprocess plus mock HTTP server prove discovery/call/disconnect and no automatic replay of side effects.
- Existing history/plugin isolation/master toggle/API/auth/hosted regressions remain passing.
- Type generation/check, Rust fmt/Clippy/workspace tests, JS/build/browser tests and native Tauri fixture smoke. No paid model calls required.
- Review all scope requirements against actual evidence, commit intended implementation and verify successful push to the current feature branch. Preserve unrelated user files.
