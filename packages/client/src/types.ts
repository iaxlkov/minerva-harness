export type ApiError = { code: string; message: string; requestId: string };
export type ApiResult<T> = {ok:true;value:T}|{ok:false;error:ApiError};
export type WorkspaceRef = {workspaceId:string};
export type Bootstrap = {protocolVersion:1;csrfToken:string;principal:{id:string};workspace:{id:string};capabilities:string[]};
export type { ContentPart, SessionInfo, SessionMessage, StartRunRequest, RunHandle, RunSnapshot,
  ProviderView as Provider, ProviderInput, PluginInfo, PluginModel,
  ShellSettings, ShellResult, RunRequest, SessionRunResult as RunReply,
  SessionBaseline, SessionEvent, JsonValue } from './generated';
import type { ContentPart, SessionInfo, SessionMessage, StartRunRequest, RunHandle, RunSnapshot,
  ProviderView as Provider, ProviderInput, PluginInfo, PluginModel,
  ShellSettings, ShellResult, RunRequest, SessionRunResult as RunReply } from './generated';
import type {WorkspaceSettings,ToolDefinition,PendingInteraction,InteractionAnswer,ContextBudget,ContextSummary,AttachmentRef,AttachmentData,McpServerView,McpServerInput} from './generated';
export type {WorkspaceSettings,ToolDefinition,PendingInteraction,InteractionAnswer,ContextBudget,ContextSummary,AttachmentRef,AttachmentData,McpServerView,McpServerInput} from './generated';
type Endpoint<P,R> = {params:P;result:R};
type Empty = Record<string,never>;
export interface Endpoints {
 'workspace.get':Endpoint<{sessionId:string},WorkspaceSettings>;
 'workspace.save':Endpoint<{sessionId:string;settings:WorkspaceSettings},WorkspaceSettings>;
 'tools.list':Endpoint<{sessionId:string},ToolDefinition[]>;
 'interactions.list':Endpoint<{sessionId:string},PendingInteraction[]>;
 'interactions.answer':Endpoint<InteractionAnswer,boolean>;
 'context.status':Endpoint<{sessionId:string},ContextBudget>;
 'context.compact':Endpoint<{sessionId:string},ContextSummary>;
 'attachments.put':Endpoint<{sessionId:string;name:string;mimeType:string;data:string},AttachmentRef>;
 'attachments.get':Endpoint<{sessionId:string;id:string},AttachmentData>;
 'mcp.list':Endpoint<Empty,McpServerView[]>;
 'mcp.save':Endpoint<{server:McpServerInput},McpServerView[]>;
 'mcp.delete':Endpoint<{id:string},McpServerView[]>;
 'plugins.models':Endpoint<Empty,PluginModel[]>;
 'plugins.ui':Endpoint<{id:string},string>;
 'plugins.call':Endpoint<{id:string;method:string;params:import('./generated.ts').JsonValue},import('./generated.ts').JsonValue>;
 'runs.start':Endpoint<StartRunRequest,RunHandle>;
 'runs.get':Endpoint<RunHandle,RunSnapshot>;
 'runs.cancel':Endpoint<RunHandle,boolean>;
 session_list:Endpoint<Empty,SessionInfo[]>; session_create:Endpoint<{id:string},void>;
 session_get:Endpoint<{id:string},SessionInfo>; session_load:Endpoint<{id:string},SessionMessage[]>;
 session_save:Endpoint<{id:string;item:SessionMessage},void>; session_set_model:Endpoint<{id:string;model:string},void>;
 session_run:Endpoint<{id:string;request:RunRequest},RunReply>; session_cancel:Endpoint<{id:string;runId:string},boolean>;
 model_providers:Endpoint<Empty,Provider[]>; save_model_provider:Endpoint<{provider:ProviderInput},Provider[]>;
 discover_models:Endpoint<{provider:ProviderInput},string[]>; delete_model_provider:Endpoint<{id:string},Provider[]>;
 list_plugins:Endpoint<Empty,PluginInfo[]>; set_plugin_enabled:Endpoint<{id:string;enabled:boolean},PluginInfo[]>;
 set_optional_plugins_enabled:Endpoint<{enabled:boolean},PluginInfo[]>;
 shell_settings:Endpoint<Empty,ShellSettings>; save_shell_settings:Endpoint<{settings:ShellSettings},ShellSettings>; run_shell_command:Endpoint<{command:string},ShellResult>;
}
export type Method = keyof Endpoints;
export type Transport = (method:Method,params:Record<string,unknown>)=>Promise<unknown>;
export interface MinervaClient {call<M extends Method>(method:M,params:Endpoints[M]['params']):Promise<Endpoints[M]['result']>}
/** Typed UI facade: parameterless endpoints may omit their empty parameter object. */
export type Invoke = <M extends Method>(method:M,...args:Record<string,never> extends Endpoints[M]['params'] ? [params?:Endpoints[M]['params']] : [params:Endpoints[M]['params']])=>Promise<Endpoints[M]['result']>;
