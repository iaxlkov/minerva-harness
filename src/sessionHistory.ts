import { transcriptContent, type ContentPart } from './transcript.ts';
import { invoke } from '@tauri-apps/api/core';
import type { ThreadHistoryAdapter, ExportedMessageRepositoryItem } from '@assistant-ui/react';

export type SessionInfo = { id:string; title:string; updatedAt:number; model:string|null };
type SessionMessage = {id:string;parentId:string|null;role:string;createdAt:string;parts:ContentPart[];extensions:Record<string,unknown>};

// Translate assistant-ui messages at the UI boundary; the session service owns
// the neutral message envelope and treats UI-only metadata as opaque extension data.
function encode(item:ExportedMessageRepositoryItem):SessionMessage {
  const {id,role,createdAt,content,...rest}=item.message;
  const parts:SessionMessage['parts']=[]; const partExtras:Record<string,unknown>[]=[];
  for(const part of content){const value=part as unknown as Record<string,unknown>;const {type,...extra}=value;
    if((type==='text'||type==='reasoning') && typeof extra.text==='string'){const {text,...metadata}=extra;parts.push({kind:type,data:{text}});partExtras.push(metadata);}
    else{parts.push({kind:'extension',data:value});partExtras.push({});}
  }
  return {id,parentId:item.parentId,role,createdAt:createdAt.toISOString(),parts,extensions:{'assistant-ui':{message:rest,parts:partExtras,runConfig:item.runConfig??null}}};
}
function decode(item:SessionMessage):ExportedMessageRepositoryItem {
  const ext=(item.extensions['assistant-ui']||{}) as {message?:Record<string,unknown>;parts?:Record<string,unknown>[];runConfig?:Record<string,unknown>|null};
  const content=transcriptContent(item.parts.map((part,index)=>part.kind==='text'||part.kind==='reasoning'?{...part,data:{...part.data,...(ext.parts?.[index]||{})}}:part));
  const execution=item.extensions.execution as {status?:string;error?:string}|undefined;
  const status=execution && execution.status!=='completed'?{type:'incomplete',reason:execution.status==='cancelled'?'cancelled':'error',...(execution.error?{error:execution.error}:{})}:{type:'complete',reason:'stop'};
  return {parentId:item.parentId,message:{...(item.role==='assistant'?{status,metadata:{custom:{},unstable_annotations:[],unstable_data:[],steps:[]}}:{}),...(ext.message||{}),...(item.role==='assistant'&&execution?{status}:{}),id:item.id,role:item.role,createdAt:new Date(item.createdAt),content},...(ext.runConfig?{runConfig:ext.runConfig}:{})} as unknown as ExportedMessageRepositoryItem;
}
export type SessionHistoryAdapter = ThreadHistoryAdapter & { beginResponse:(id:string,parent:string)=>void; completeResponse:(id:string)=>void; recoverResponse:(id:string)=>Promise<ExportedMessageRepositoryItem|null> };
export function sessionHistory(id:string,onSaved:()=>void,onError:(message:string)=>void,ipc:typeof invoke=invoke):SessionHistoryAdapter {
  const transientParents=new Map<string,string>();
  const save=async(item:ExportedMessageRepositoryItem)=>{
    if(item.message.role!=='user')return; // The backend already committed the model result.
    let parentId=item.parentId;
    while(parentId && transientParents.has(parentId))parentId=transientParents.get(parentId)!;
    try {await ipc('session_save',{id,item:encode({...item,parentId})});onSaved();}
    catch(error){onError(`Не удалось сохранить сообщение: ${String(error)}`);throw error;}
  };
  return {async recoverResponse(responseId){
    const saved=(await ipc<SessionMessage[]>('session_load',{id})).find(m=>m.id===responseId);
    if(!saved)return null;
    transientParents.delete(responseId);onSaved();return decode(saved);
  },beginResponse:(id,parent)=>{transientParents.set(id,parent);},completeResponse:id=>{transientParents.delete(id);},scopeId:id,async load(){try{return {messages:(await ipc<SessionMessage[]>('session_load',{id})).map(decode)};}catch(error){onError(`Не удалось открыть историю: ${String(error)}`);throw error;}},append:save,update:save};
}
