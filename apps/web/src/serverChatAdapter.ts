import type { ChatModelAdapter } from '@assistant-ui/react';
import type { ChatAdapterConfig } from './chatAdapter.ts';
import { transcriptContent } from './transcript.ts';

const pause=(ms:number)=>new Promise<void>(resolve=>setTimeout(resolve,ms));
function retryable(error:unknown):boolean{
  return !!error && typeof error==='object' && 'code' in error
    && ['network_error','http_502','http_503','http_504'].includes(String(error.code));
}

/** An observer may disappear without changing ownership of the accepted run. */
export function serverChatAdapter(config:ChatAdapterConfig):ChatModelAdapter {
  return {async *run({messages,abortSignal,unstable_assistantMessageId,unstable_getMessage}){
    const runId=unstable_assistantMessageId??unstable_getMessage().id;
    const ref={sessionId:config.sessionId,runId};
    let settled=false,cancelling=false;
    const cancel=async()=>{
      if(cancelling)return;cancelling=true;
      const deadline=Date.now()+30000;
      while(!settled){
        try{if(await config.invoke('runs.cancel',ref))return;}
        catch(error){if(!retryable(error)||Date.now()>deadline){config.onError(`Не удалось подтвердить отмену: ${String(error)}`);return;}}
        if(Date.now()>deadline){config.onError('Сервер не подтвердил отмену. Проверьте состояние задачи после подключения.');return;}
        await pause(200);
      }
    };
    const onAbort=()=>{void cancel();};
    config.onRunning(true);
    try{
      abortSignal.throwIfAborted();
      const last=messages.at(-1);
      if(!last||last.role!=='user')throw Error('Нужно сообщение пользователя.');
      const text=last.content.filter(part=>part.type==='text').map(part=>part.text).join('\n');
      if(!text.trim())throw Error('Нужно непустое сообщение пользователя.');
      config.history.beginResponse(runId,last.id);
      abortSignal.addEventListener('abort',onAbort,{once:true});
      const request={sessionId:config.sessionId,requestId:runId,parentId:config.history.resolveParent(messages.at(-2)?.id??null),message:{id:last.id,text},model:config.model,options:config.options};
      const startDeadline=Date.now()+30000;
      while(true){
        try{await config.invoke('runs.start',request);break;}
        catch(error){if(!retryable(error)||Date.now()>startDeadline)throw error;await pause(500);}
      }
      let lastContent='',offlineSince=0;
      while(true){
        let snapshot;
        try{snapshot=await config.invoke('runs.get',ref);offlineSince=0;}
        catch(error){
          if(!retryable(error))throw error;
          offlineSince ||= Date.now();
          if(Date.now()-offlineSince>30000)throw Error('Связь с сервером потеряна. Задача продолжает выполняться; откройте диалог после подключения.');
          await pause(500);continue;
        }
        const terminal=snapshot.status!=='running';
        const serialized=JSON.stringify(snapshot.parts);
        if(serialized!==lastContent||terminal){
          lastContent=serialized;
          const content=transcriptContent(snapshot.parts);
          if(terminal){
            if(snapshot.parts.length||snapshot.status==='completed')config.history.completeResponse(runId);
            config.onSaved();
            if(snapshot.status==='completed')yield {content};
            else{
              if(snapshot.error&&snapshot.status!=='cancelled')config.onError(snapshot.error);
              yield {content,status:{type:'incomplete',reason:snapshot.status==='cancelled'?'cancelled':'error',...(snapshot.error?{error:snapshot.error}:{})}};
            }
            break;
          }
          if(content.length)yield {content};
        }
        await pause(200);
      }
    }catch(error){
      if(!abortSignal.aborted)config.onError(`Не удалось получить ответ: ${String(error)}`);
      try{
        const saved=await config.history.recoverResponse(runId);
        if(saved?.message.role==='assistant'){yield {content:saved.message.content,status:saved.message.status};return;}
      }catch(recoveryError){config.onError(`Не удалось восстановить ход: ${String(recoveryError)}`);}
      throw error;
    }finally{
      settled=true;abortSignal.removeEventListener('abort',onAbort);config.onRunning(false);
    }
  }};
}
