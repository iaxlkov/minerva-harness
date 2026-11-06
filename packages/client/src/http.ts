import {createClient,MinervaError} from './index.ts';
import type {Bootstrap,MinervaClient} from './types.ts';
type Options = {fetch?:typeof fetch;baseUrl?:string;workspaceId?:string;location?:Pick<Location,'href'>;history?:Pick<History,'state'|'replaceState'>};
const invalid = (message:string)=>new MinervaError({code:'invalid_response',message,requestId:''});
export async function createHttpClient(options:Options={}):Promise<MinervaClient & {bootstrap:Bootstrap}> {
 const request=options.fetch??globalThis.fetch;const base=options.baseUrl??'';
 async function json(path:string,init:RequestInit,allowEmpty=false){
  let response:Response;
  try{response=await request(`${base}/api/v1/${path}`,{...init,credentials:'same-origin'});}
  catch{throw new MinervaError({code:'network_error',message:'Unable to reach server',requestId:''});}
  if(!response.ok){
   const failure=await response.clone().json().catch(()=>null);
   if(failure?.ok===false&&typeof failure.error?.code==='string'&&typeof failure.error?.message==='string'&&typeof failure.error?.requestId==='string')throw new MinervaError(failure.error);
   throw new MinervaError({code:`http_${response.status}`,message:`Server returned HTTP ${response.status}`,requestId:response.headers.get('X-Request-ID')??''});
  }
  if(allowEmpty&&response.status===204)return null;
  try{return await response.json();}catch{throw invalid('Server returned invalid JSON');}
 }
 const location=options.location??globalThis.location;
 const history=options.history??globalThis.history;
 if(location){
  const launchUrl=new URL(location.href);
  const fragment=new URLSearchParams(launchUrl.hash.slice(1));
  if(fragment.has('minerva-token')){
   const token=fragment.get('minerva-token')!;
   fragment.delete('minerva-token');launchUrl.hash=fragment.toString();
   if(!history)throw invalid('Cannot clear launch token from URL');
   history.replaceState(history.state,'',launchUrl.toString());
   await json('auth/exchange',{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify({token})},true);
  }
 }
 const bootstrap=await json(`bootstrap${options.workspaceId?`?workspaceId=${encodeURIComponent(options.workspaceId)}`:''}`,{method:'GET'});
 if(bootstrap?.protocolVersion!==1)throw new MinervaError({code:'unsupported_protocol',message:'Unsupported server protocol version',requestId:''});
 if(typeof bootstrap.csrfToken!=='string'||!bootstrap.csrfToken||typeof bootstrap.principal?.id!=='string'||typeof bootstrap.workspace?.id!=='string'||!Array.isArray(bootstrap.capabilities)||!bootstrap.capabilities.every((v:unknown)=>typeof v==='string'))throw invalid('Invalid bootstrap response');
 const client=createClient(async(method,params)=>{
  const result=await json('rpc',{method:'POST',headers:{'Content-Type':'application/json','X-Minerva-CSRF':bootstrap.csrfToken,'X-Minerva-Workspace':bootstrap.workspace.id},body:JSON.stringify({protocolVersion:1,method,params})});
  if(result?.ok===true&&Object.hasOwn(result,'value'))return result.value;
  if(result?.ok===false&&typeof result.error?.code==='string'&&typeof result.error?.message==='string'&&typeof result.error?.requestId==='string')throw new MinervaError(result.error);
  throw invalid('Invalid RPC result');
 });
 return Object.assign(client,{bootstrap:bootstrap as Bootstrap});
}
