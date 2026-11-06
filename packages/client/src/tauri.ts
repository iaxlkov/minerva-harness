import {invoke} from '@tauri-apps/api/core';
import {createClient,MinervaError} from './index.ts';
type TauriTransport=(command:string,params:Record<string,unknown>)=>Promise<unknown>;
export function createTauriClient(transport:TauriTransport=invoke){
 return createClient(async(method,params)=>{
  try{
   if(method==='runs.start')return await transport('run_start',{request:params});
   if(method==='runs.get')return await transport('run_get',params);
   if(method==='runs.cancel')return await transport('run_cancel',params);
   if(method==='plugins.call')return await transport('plugin_call',params);
   return await transport(method,params);
  }catch(error){
   if(error instanceof MinervaError)throw error;
   if(error&&typeof error==='object'&&'code' in error&&'message' in error&&'requestId' in error)throw new MinervaError(error as {code:string;message:string;requestId:string});
   throw new MinervaError({code:'desktop_error',message:String(error),requestId:''});
  }
 });
}
