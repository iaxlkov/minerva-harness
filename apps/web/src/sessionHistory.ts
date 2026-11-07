import type { SessionInfo, SessionMessage } from '../../../packages/client/src/types.ts';
import { transcriptContent } from './transcript.ts';
import { invoke } from './client.ts';
import type { ThreadHistoryAdapter, ExportedMessageRepositoryItem } from '@assistant-ui/react';
import {jsonObject,jsonSnapshot} from './json.ts';


// Translate assistant-ui messages at the UI boundary; the session service owns
// the neutral message envelope and treats UI-only metadata as opaque extension data.
function encode(item:ExportedMessageRepositoryItem):SessionMessage {
  const {id,role,createdAt,content,...rest}=item.message;
  const parts:SessionMessage['parts']=[]; const partExtras:Record<string,unknown>[]=[];
  for(const part of content){const value=part as unknown as Record<string,unknown>;const {type,...extra}=value;
    if((type==='text'||type==='reasoning') && typeof extra.text==='string'){const {text,...metadata}=extra;parts.push({kind:type,data:{text}});partExtras.push(metadata);}
    else{parts.push({kind:'extension',data:jsonSnapshot(value)});partExtras.push({});}
  }
  return {id,parentId:item.parentId,role,createdAt:createdAt.toISOString(),parts,extensions:{'assistant-ui':jsonSnapshot({message:rest,parts:partExtras,runConfig:item.runConfig??null})}};
}
export function decodeSessionMessage(item:SessionMessage):ExportedMessageRepositoryItem {
  const ext=(item.extensions['assistant-ui']||{}) as {message?:Record<string,unknown>;parts?:Record<string,unknown>[];runConfig?:Record<string,unknown>|null};
  const content=transcriptContent(item.parts.map((part,index)=>part.kind==='text'||part.kind==='reasoning'?{...part,data:jsonSnapshot({...jsonObject(part.data),...(ext.parts?.[index]||{})})}:part));
  const execution=item.extensions.execution as {status?:string;error?:string}|undefined;
  const status=execution?.status==='running'?{type:'running'}:execution && execution.status!=='completed'?{type:'incomplete',reason:execution.status==='cancelled'?'cancelled':'error',...(execution.error?{error:execution.error}:{})}:{type:'complete',reason:'stop'};
  return {parentId:item.parentId,message:{...(item.role==='assistant'?{status,metadata:{custom:{},unstable_annotations:[],unstable_data:[],steps:[]}}:{}),...(ext.message||{}),...(item.role==='assistant'&&execution?{status}:{}),id:item.id,role:item.role,createdAt:new Date(item.createdAt),content},...(ext.runConfig?{runConfig:ext.runConfig}:{})} as unknown as ExportedMessageRepositoryItem;
}
export type SessionHistoryAdapter = ThreadHistoryAdapter & { beginResponse:(id:string,parent:string)=>void; completeResponse:(id:string)=>void; recoverResponse:(id:string)=>Promise<ExportedMessageRepositoryItem|null>; resolveParent:(id:string|null)=>string|null };
export function sessionHistory(id:string,onSaved:()=>void,onError:(message:string)=>void,ipc:typeof invoke=invoke,options:{serverOwned?:boolean}={}):SessionHistoryAdapter {
  const transientParents=new Map<string,string>();
  const resolveParent=(parentId:string|null)=>{while(parentId&&transientParents.has(parentId))parentId=transientParents.get(parentId)!;return parentId;};
  const save=async(item:ExportedMessageRepositoryItem)=>{
    if(options.serverOwned)return; // runs.start owns the atomic user input and accepted run.
    if(item.message.role!=='user')return; // The backend already committed the model result.
    let parentId=item.parentId;
    while(parentId && transientParents.has(parentId))parentId=transientParents.get(parentId)!;
    try {await ipc('session_save',{id,item:encode({...item,parentId})});onSaved();}
    catch(error){onError(`Не удалось сохранить сообщение: ${String(error)}`);throw error;}
  };
  return {resolveParent,async recoverResponse(responseId){
    const saved=(await ipc('session_load',{id})).find(m=>m.id===responseId);
    if(!saved)return null;
    transientParents.delete(responseId);onSaved();return decodeSessionMessage(saved);
  },beginResponse:(id,parent)=>{transientParents.set(id,parent);},completeResponse:id=>{transientParents.delete(id);},scopeId:id,async load(){try{return {messages:(await ipc('session_load',{id})).map(decodeSessionMessage)};}catch(error){onError(`Не удалось открыть историю: ${String(error)}`);throw error;}},append:save,update:save};
}
