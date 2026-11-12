import type { ApiError, Invoke, MinervaClient, Transport } from './types.ts';
export type * from './types.ts';
export class MinervaError extends Error implements ApiError {
 code:string; requestId:string;
 constructor(error:ApiError){super(error.message);this.name='MinervaError';this.code=error.code;this.requestId=error.requestId;}
}
export const methods = ['sessions.rename','sessions.state','sessions.fork','session.snapshot','mh-editor.list','mh-editor.read','mh-editor.save','mh-editor.search','mh-terminal.open','mh-terminal.write','mh-terminal.read','mh-terminal.resize','mh-terminal.close','mh-lsp.start','mh-lsp.status','mh-lsp.stop','mh-lsp.open','mh-lsp.change','mh-lsp.close','mh-lsp.query','mh-lsp.diagnostics','workspaces.list','workspaces.rename','workspaces.remove','workspaces.create','workspace.get','workspace.save','tools.list','interactions.list','interactions.answer','context.status','context.compact','attachments.put','attachments.get','mcp.list','mcp.save','mcp.delete','plugins.call','plugins.models','plugins.ui','runs.start','runs.get','runs.cancel','session_list','session_create','session_get','session_load','session_save','session_set_model','session_run','session_cancel','model_providers','save_model_provider','discover_models','delete_model_provider','list_plugins','set_plugin_enabled','set_optional_plugins_enabled','shell_settings','save_shell_settings','run_shell_command'] as const;
export function createClient(transport:Transport):MinervaClient {
 return {async call(method,params){
  if(!(methods as readonly string[]).includes(method))throw new MinervaError({code:'unknown_method',message:`Unknown method: ${method}`,requestId:''});
  return await transport(method,params) as never;
 }};
}
let configured:MinervaClient|undefined;
export function configureClient(client:MinervaClient){configured=client;}
export const invoke:Invoke = async (method,...args)=>{
 if(!configured)throw new MinervaError({code:'client_not_initialized',message:'Client has not been initialized',requestId:''});
 return configured.call(method,(args[0]??{}) as never);
};
