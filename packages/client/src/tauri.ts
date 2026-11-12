import {invoke} from '@tauri-apps/api/core';
import {createClient,MinervaError} from './index.ts';
type TauriTransport=(command:string,params:Record<string,unknown>)=>Promise<unknown>;
export function createTauriClient(transport:TauriTransport=invoke){
 return createClient(async(method,params)=>{
  try{
   if(['sessions.rename','sessions.state','sessions.fork','session.snapshot','mh-editor.list','mh-editor.read','mh-editor.save','mh-editor.search','mh-terminal.open','mh-terminal.write','mh-terminal.read','mh-terminal.resize','mh-terminal.close','mh-lsp.start','mh-lsp.status','mh-lsp.stop','mh-lsp.open','mh-lsp.change','mh-lsp.close','mh-lsp.query','mh-lsp.diagnostics','workspaces.list','workspaces.rename','workspaces.remove','workspaces.create','workspace.get','workspace.save','tools.list','interactions.list','interactions.answer','context.status','context.compact','attachments.put','attachments.get','mcp.list','mcp.save','mcp.delete'].includes(method))return await transport('harness_rpc',{method,params});
   if(method==='runs.start')return await transport('run_start',{request:params});
   if(method==='runs.get')return await transport('run_get',params);
   if(method==='runs.cancel')return await transport('run_cancel',params);
   if(method==='plugins.call')return await transport('plugin_call',params);
   if(method==='plugins.models')return await transport('plugin_models',params);
   if(method==='plugins.ui')return await transport('plugin_ui',params);
   return await transport(method,params);
  }catch(error){
   if(error instanceof MinervaError)throw error;
   if(error&&typeof error==='object'&&'code' in error&&'message' in error&&'requestId' in error)throw new MinervaError(error as {code:string;message:string;requestId:string});
   throw new MinervaError({code:'desktop_error',message:String(error),requestId:''});
  }
 });
}
