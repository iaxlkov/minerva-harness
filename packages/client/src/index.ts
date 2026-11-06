import type { ApiError, Invoke, MinervaClient, Transport } from './types.ts';
export type * from './types.ts';
export class MinervaError extends Error implements ApiError {
 code:string; requestId:string;
 constructor(error:ApiError){super(error.message);this.name='MinervaError';this.code=error.code;this.requestId=error.requestId;}
}
export const methods = ['plugins.call','runs.start','runs.get','runs.cancel','session_list','session_create','session_get','session_load','session_save','session_set_model','session_run','session_cancel','model_providers','save_model_provider','discover_models','delete_model_provider','codex_models','codex_status','codex_quota','codex_login','codex_login_status','list_plugins','set_plugin_enabled','shell_settings','save_shell_settings','run_shell_command'] as const;
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
