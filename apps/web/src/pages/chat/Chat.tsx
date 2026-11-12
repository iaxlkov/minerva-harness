import {object,toolState} from '../../execution/toolModel';
import {messageQueueOptions,QueueComposerRoot,QueueComposerSend} from '../../../../../plugins/mh-message-queue/ui';
import {NavigationShell} from '../../app/NavigationShell';
import { ScrollArea } from '../../components/ui/scroll-area';
import {AttachmentPicker} from '../../harness/AttachmentPicker';
import {ComposerAttachments} from '../../components/assistant-ui/elements/attachment';
import {SessionFile} from '../../harness/SessionFile';
import {SessionImage} from '../../harness/SessionImage';
import {attachmentAdapter} from '../../harness/attachments';
import {AttachmentPrimitive,ActionBarPrimitive} from '@assistant-ui/react';
import {Check,Copy} from 'lucide-react';
import {Tooltip,TooltipContent,TooltipTrigger} from '../../components/ui/tooltip';
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../../client.ts';
import {
  AssistantRuntimeProvider, useLocalRuntime, ThreadPrimitive, AuiIf,
  MessagePrimitive, ComposerPrimitive, type ChatModelAdapter,
} from '@assistant-ui/react';
import {ComposerBar, ComposerToolbar, ComposerActions, ComposerSend, ComposerModelTrigger} from '../../components/assistant-ui/elements/composer';
import {useAuiState} from '@assistant-ui/react';
import { Button } from '../../components/ui/button';
import { DropdownMenu, DropdownMenuTrigger, DropdownMenuContent, DropdownMenuGroup, DropdownMenuLabel, DropdownMenuRadioGroup, DropdownMenuRadioItem, DropdownMenuSeparator, DropdownMenuSub, DropdownMenuSubTrigger, DropdownMenuSubContent } from '../../components/ui/dropdown-menu';
import { Brain } from 'lucide-react';
const effortLabels:Record<string,string>={default:'По умолчанию',off:'Выключено',minimal:'Минимальный',low:'Низкий',medium:'Средний',high:'Высокий',xhigh:'Очень высокий',max:'Максимальный'};
function readEffort(model:string){try{return localStorage.getItem(`minerva.reasoning.${model}`)??'default';}catch{return 'default';}}
/** Reasoning choices the providers plugin reports for an `api:<provider>/<model>` id; empty when the model has none. */
function reasoningChoices(providers:Provider[],model:string){const match=model.match(/^api:([^/]+)\/(.+)$/);return match?providers.find(p=>p.id===match[1])?.reasoning?.[match[2]]??[]:[];}
import '../../style.css';
import type { Provider, SessionInfo, PluginModel } from '../../../../../packages/client/src/types.ts';
import { sessionHistory,decodeSessionMessage } from '../../sessionHistory';
import {useRemoteSession} from '../../app/useRemoteSession';
import {useRunningSession} from '../../app/keepAwake';
import {ThreadSearch} from './ThreadSearch';
import {codeSlotKey, splitCodeBlocks} from './codeBlocks';
import { chatAdapter } from '../../chatAdapter';
import { ToolHistory } from '../../ToolHistory';
import {ImageGenerating} from '../../components/assistant-ui/elements/image';

import {UiSlot,workspaceScope,sessionScope} from "../../app/plugins";
const SessionIdContext=React.createContext('');
const SessionScopeContext=React.createContext(sessionScope("unselected"));
function PluginTool(props:React.ComponentProps<typeof ToolHistory>){
 const scope=React.useContext(SessionScopeContext),state=toolState(props).status;
 const approval=scope.kind==='session'&&state==='requires-approval'&&<UiSlot slot="tool.interactions" scope={workspaceScope} componentProps={{sessionId:scope.sessionId,callId:props.toolCallId}}/>;
 if(props.toolName==='generate_image'){
  if(state==='completed')return null;
  if(state==='failed'||state==='cancelled'||state==='interrupted'){
   const output=object(object(props.result).output),error=typeof output.error==='string'?output.error:'';
   return <p className="image-generation-error" role="alert">{state==='cancelled'?'Создание изображения отменено.':error||'Не удалось создать изображение.'}</p>;
  }
  return <><ImageGenerating callId={props.toolCallId} awaitingApproval={state==='requires-approval'}/>{approval}</>;
 }
 return <><UiSlot slot="tool.renderers" scope={scope} entryId={props.toolName} componentProps={{...props}} fallback={<ToolHistory {...props}/>}/>{approval}</>;
}
function TextPart({text}:{text:string}){
 const sessionId=React.useContext(SessionIdContext);
 return <>{splitCodeBlocks(text).map((segment,i)=>segment.kind==='text'?<React.Fragment key={i}>{segment.text}</React.Fragment>:<UiSlot key={i} slot="message.code" scope={workspaceScope} entryId={codeSlotKey(segment.language)} componentProps={{code:segment.code,language:segment.language,sessionId}} fallback={<pre className="message-code" data-language={segment.language||undefined}>{segment.code}</pre>}/>)}</>;
}
function HistoryFile(props:Omit<React.ComponentProps<typeof SessionFile>,'sessionId'>){return <SessionFile {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function HistoryImage(props:Omit<React.ComponentProps<typeof SessionImage>,'sessionId'>){return <SessionImage {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function Message({ role }: { role: 'user' | 'assistant' }) {
  const scope=React.useContext(SessionScopeContext);
  const standard=<MessagePrimitive.Parts components={{Text:TextPart,Image:HistoryImage,File:HistoryFile,tools:{Fallback:PluginTool},data:{Fallback:({data})=><details className="tool-history"><summary>Событие провайдера</summary><ScrollArea asChild><pre>{JSON.stringify(data,null,2)}</pre></ScrollArea></details>},Reasoning:({text})=><details className="message-reasoning"><summary>Сводка рассуждений</summary><div>{text}</div></details>}}/>;
  const messageId=useAuiState(s=>s.message.id);
  return <MessagePrimitive.Root className={`message ${role}`} data-role={role} data-message-id={messageId}>
    <div className="message-body">
    <div className="message-content"><div className="message-text"><UiSlot slot="message.renderers" scope={scope} entryId={role} componentProps={{children:standard}} fallback={standard}/><AuiIf condition={s=>s.message.status?.type==='incomplete'}><div className="message-interrupted">Ход не завершён. Сохранённые действия показаны выше.</div></AuiIf></div></div>
    {/* The row keeps its height so hover-revealed actions do not shift the thread. */}
    <div className="message-actions-row"><ActionBarPrimitive.Root hideWhenRunning autohide="always" className="message-actions" aria-label="Действия с сообщением">
      <Tooltip><TooltipTrigger asChild><ActionBarPrimitive.Copy asChild copiedDuration={2000}><Button variant="ghost" size="icon-sm" className="message-action" aria-label="Копировать сообщение"><Copy className="message-copy-idle"/><Check className="message-copy-done"/></Button></ActionBarPrimitive.Copy></TooltipTrigger><TooltipContent className="oe-studio">Копировать</TooltipContent></Tooltip>
      <UiSlot slot="message.actions" scope={scope} componentProps={{role}}/>
    </ActionBarPrimitive.Root></div>
    </div>
  </MessagePrimitive.Root>;
}

function Thread({ model, providers, pluginModels, onSelect,effort,onEffort,remoteRunning,onRemoteCancel }: { model: string; configured: boolean; providers: Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void;effort:string;onEffort:(value:string)=>void;remoteRunning?:boolean;onRemoteCancel?:()=>void }) {
  const choices=reasoningChoices(providers,model);
  const scope=React.useContext(SessionScopeContext);
  const running=useAuiState(state=>state.thread.isRunning);
  const idle=useAuiState(state=>!state.composer.text.trim()&&!state.composer.attachments.length);
  const empty=useAuiState(state=>state.thread.isEmpty);
  return <ThreadPrimitive.Root className={empty?"thread thread-empty":"thread"}>
    <ThreadSearch/>
    <ScrollArea asChild><ThreadPrimitive.Viewport className="viewport">
      <AuiIf condition={(state) => state.thread.isEmpty}><div className="empty">
        <h1>С чего начнём?</h1>
      </div></AuiIf>
      <div className="messages"><ThreadPrimitive.Messages>{({ message }) => <Message role={message.role === "user" ? "user" : "assistant"}/>}</ThreadPrimitive.Messages></div>
    </ThreadPrimitive.Viewport></ScrollArea>
    <div className="composer-wrap">
      <QueueComposerRoot className="oe-relative">
        <UiSlot slot="composer.queue" scope={workspaceScope} componentProps={{}}/>
        <ComposerBar className="oe-bg-secondary chat-composer">
        <ComposerAttachments className="harness-attachments"/><ScrollArea asChild><ComposerPrimitive.Input rows={1} disabled={!!remoteRunning} className="placeholder:oe-text-muted-foreground oe-min-h-8 oe-w-full oe-resize-none oe-text-foreground oe-border-0 oe-bg-transparent oe-px-3 oe-text-[15px] oe-outline-none focus-visible:oe-outline-none" placeholder="Напишите сообщение…" aria-label="Сообщение ассистенту" /></ScrollArea>
        <ComposerToolbar>
          <div className="composer-access-actions">
          <AttachmentPicker disabled={!!remoteRunning} renderMenu={onClose=><UiSlot slot="composer.menu" scope={workspaceScope} componentProps={{sessionId:scope.kind==='session'?scope.sessionId:'',disabled:!!remoteRunning,onClose}}/>}/>
          </div>
          <ComposerActions>
          <UiSlot slot="composer.actions" scope={scope} componentProps={{model,onSelect}}/><AuiIf condition={state=>(!state.thread.isRunning||!idle)&&!remoteRunning}><QueueComposerSend asChild><ComposerSend streaming={false} idle={idle} aria-label="Отправить сообщение"/></QueueComposerSend></AuiIf><AuiIf condition={state=>state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Cancel asChild><ComposerSend streaming idle={false} aria-label="Остановить ответ"/></ComposerPrimitive.Cancel></AuiIf><AuiIf condition={()=>!!remoteRunning}><ComposerSend streaming idle={false} aria-label="Остановить ответ" onClick={onRemoteCancel}/></AuiIf>
          </ComposerActions>
        </ComposerToolbar>
        </ComposerBar>
      </QueueComposerRoot>
      <div className="composer-footer">
        <div className="composer-footer-slot">{scope.kind==='session'&&<UiSlot slot="composer.access" scope={workspaceScope} componentProps={{sessionId:scope.sessionId,disabled:!!remoteRunning||running}}/>}<UiSlot slot="composer.footer" scope={scope} componentProps={{model}}/></div>
        <DropdownMenu>
          <DropdownMenuTrigger asChild><ComposerModelTrigger model={(model.replace(/^api:[^/]+\//, "") || "Выберите модель")+(choices.length&&effort!=='default'?` · ${effortLabels[effort]??effort}`:'')} aria-label="Модель" /></DropdownMenuTrigger>
          <DropdownMenuContent align="end" side="top" className="composer-model-menu">
            <DropdownMenuRadioGroup value={model} onValueChange={onSelect}>
              {providers.filter(provider=>provider.models.length).map(provider => <DropdownMenuGroup key={provider.id}>
                <DropdownMenuLabel>{provider.name}</DropdownMenuLabel>
                {provider.models.map(name => <DropdownMenuRadioItem key={name} value={`api:${provider.id}/${name}`}>{name}</DropdownMenuRadioItem>)}
              </DropdownMenuGroup>)}
              {[...new Set(pluginModels.map(m=>m.group))].map(group=><DropdownMenuGroup key={`plugin:${group}`}><DropdownMenuLabel>{group}</DropdownMenuLabel>{pluginModels.filter(m=>m.group===group).map(m=><DropdownMenuRadioItem key={m.id} value={m.id}>{m.name}</DropdownMenuRadioItem>)}</DropdownMenuGroup>)}
            </DropdownMenuRadioGroup>
            {!providers.some(provider=>provider.models.length)&&!pluginModels.length&&<DropdownMenuLabel>Модели не настроены. Добавьте провайдера в разделе «Настройки → Модели».</DropdownMenuLabel>}
            {choices.length>0&&<><DropdownMenuSeparator/><DropdownMenuSub><DropdownMenuSubTrigger><Brain/>Размышление<span className="ds-dropdown-meta">{effortLabels[effort]??effort}</span></DropdownMenuSubTrigger>
              <DropdownMenuSubContent><DropdownMenuRadioGroup value={effort} onValueChange={onEffort}>{choices.map(choice=><DropdownMenuRadioItem key={choice} value={choice}>{effortLabels[choice]??choice}</DropdownMenuRadioItem>)}</DropdownMenuRadioGroup></DropdownMenuSubContent>
            </DropdownMenuSub></>}
            <UiSlot slot="composer.model.menu" scope={scope} componentProps={{model,onSelect}}/>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
      {remoteRunning&&<AuiIf condition={state=>!state.thread.isRunning}><div role="status">Задача выполняется на сервере…</div></AuiIf>}
    </div>
  </ThreadPrimitive.Root>;
}

import {models} from "../../app/models";

function SessionChat({ model, configured, providers, pluginModels, onSelect, onProviders, activePlugins, onNew, session, sessions, onSession, onSaved, onError }: { model: string; configured: boolean; providers:Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void; onProviders:(p:Provider[])=>void; activePlugins:string[]; onNew:(workspaceId?:string)=>void; session:SessionInfo; sessions:SessionInfo[]; onSession:(session:SessionInfo)=>void; onSaved:()=>void; onError:(message:string)=>void }) {
  const [running,setRunningState]=useState(false);
  const attached=useRef(true);
  // Mark the observer detached before assistant-ui's passive cleanup aborts its adapter.
  useLayoutEffect(()=>{attached.current=true;return ()=>{attached.current=false;};},[]);
  const localRun=useRef(false);
  const setRunning=(running:boolean)=>{localRun.current=running;if(attached.current)setRunningState(running);};
  const history=useMemo(()=>sessionHistory(session.id,onSaved,onError,invoke,{serverOwned:true}),[session.id]);
  // Reasoning level is remembered per model; a level the model no longer lists falls back to the provider default.
  const [savedEffort,setSavedEffort]=useState(()=>readEffort(model));
  useEffect(()=>setSavedEffort(readEffort(model)),[model]);
  const effort=reasoningChoices(providers,model).includes(savedEffort)?savedEffort:'default';
  const selectEffort=(value:string)=>{setSavedEffort(value);try{localStorage.setItem(`minerva.reasoning.${model}`,value);}catch{}};
  const adapter = useMemo<ChatModelAdapter>(() => {
    return chatAdapter({invoke,serverOwned:true,isAttached:()=>attached.current,sessionId:session.id,model,history,onRunning:setRunning,onSaved,onError,
      options:effort==='default'?{}:{reasoningEffort:effort}});
  }, [model, session.id, history, effort]);
  const attachments=useMemo(()=>attachmentAdapter(session.id,invoke),[session.id]);
  const runtime = useLocalRuntime(adapter,{...messageQueueOptions,adapters:{history,attachments}});
  const remote=useRemoteSession(session.id);
  useRunningSession(session.id,running||!!remote.activeRun);
  useEffect(()=>{
    if(remote.status!=='ready')return;
    if(remote.session)models.sessions.update(remote.session);
    // Imported running messages are observations, not a locally owned adapter run.
    if(!localRun.current){
      runtime.thread.import({messages:remote.messages.map(decodeSessionMessage)});
      setRunningState(!!remote.activeRun);
    }
  },[remote,runtime,running]);
  const cancelRemote=()=>{if(remote.activeRun)void invoke('runs.cancel',{sessionId:session.id,runId:remote.activeRun.runId}).catch(error=>onError(`Не удалось отменить задачу: ${String(error)}`));};
  return <SessionIdContext.Provider value={session.id}><SessionScopeContext.Provider value={sessionScope(session.id)}><AssistantRuntimeProvider runtime={runtime}>

    <UiSlot slot="session.controls" scope={workspaceScope} componentProps={{sessionId:session.id,running:running||!!remote.activeRun}}/><UiSlot slot="conversation.views" scope={sessionScope(session.id)} entryId="default" componentProps={{session,model,configured,providers,pluginModels,onSelect}} fallback={<Thread model={model} configured={configured} providers={providers} pluginModels={pluginModels} onSelect={onSelect} effort={effort} onEffort={selectEffort} remoteRunning={!!remote.activeRun&&!localRun.current} onRemoteCancel={cancelRemote}/>}/><UiSlot slot="workspace.panels" scope={workspaceScope} componentProps={{session}}/>

  </AssistantRuntimeProvider></SessionScopeContext.Provider></SessionIdContext.Provider>;
}

export function Chat(props:React.ComponentProps<typeof SessionChat>){
 return <NavigationShell session={props.session} sessions={props.sessions} onSession={props.onSession} onNew={props.onNew} providers={props.providers} onProviders={props.onProviders} activePlugins={props.activePlugins}><SessionChat key={props.session.id} {...props}/></NavigationShell>;
}
