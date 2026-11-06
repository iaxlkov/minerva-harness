import { invoke } from '@tauri-apps/api/core';
import type { ThreadHistoryAdapter, ExportedMessageRepositoryItem } from '@assistant-ui/react';

export type SessionInfo = { id:string; title:string; updatedAt:number; model:string|null };
type IntegrationState = {namespace:string;value:unknown};
type SessionMessage = {id:string;parentId:string|null;role:string;createdAt:string;parts:{kind:string;data:unknown}[];extensions:Record<string,unknown>};
export type PendingIntegration = { current:IntegrationState|null };

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
  const content=item.parts.map((part,index)=>part.kind==='extension'?part.data:{type:part.kind,...(part.data as object),...(ext.parts?.[index]||{})});
  return {parentId:item.parentId,message:{...(ext.message||{}),id:item.id,role:item.role,createdAt:new Date(item.createdAt),content},...(ext.runConfig?{runConfig:ext.runConfig}:{})} as unknown as ExportedMessageRepositoryItem;
}
export function sessionHistory(id:string,onSaved:()=>void,onError:(message:string)=>void,pending:PendingIntegration):ThreadHistoryAdapter {
  const save=async(item:ExportedMessageRepositoryItem)=>{
    const integration=item.message.role==='assistant'?pending.current:null;
    const commitIntegration=integration?{...integration,value:{...(integration.value as object),anchorMessageId:item.message.id}}:null;
    try {await invoke('session_save',{id,item:encode(item),integration:commitIntegration});if(integration&&pending.current===integration)pending.current=null;onSaved();}
    catch(error){onError(`Не удалось сохранить сообщение: ${String(error)}`);throw error;}
  };
  return {scopeId:id,async load(){try{return {messages:(await invoke<SessionMessage[]>('session_load',{id})).map(decode)};}catch(error){onError(`Не удалось открыть историю: ${String(error)}`);throw error;}},append:save,update:save};
}
