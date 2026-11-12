import {object,toolState} from '../../execution/toolModel';
import {messageQueueOptions,QueueComposerRoot,QueueComposerSend} from '../../../../../plugins/mh-message-queue/ui';
import {NavigationShell} from '../../app/NavigationShell';
import { ScrollArea } from '../../components/ui/scroll-area';
import {AttachmentPicker} from '../../harness/AttachmentPicker';
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
import { Select, SelectGroup, SelectLabel, SelectTrigger, SelectContent, SelectItem } from '../../components/ui/select';
import '../../style.css';
import type { Provider, SessionInfo, PluginModel } from '../../../../../packages/client/src/types.ts';
import { sessionHistory,decodeSessionMessage } from '../../sessionHistory';
import {useRemoteSession} from '../../app/useRemoteSession';
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
function HistoryFile(props:Omit<React.ComponentProps<typeof SessionFile>,'sessionId'>){return <SessionFile {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function HistoryImage(props:Omit<React.ComponentProps<typeof SessionImage>,'sessionId'>){return <SessionImage {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function Message({ role }: { role: 'user' | 'assistant' }) {
  const scope=React.useContext(SessionScopeContext);
  const standard=<MessagePrimitive.Parts components={{Image:HistoryImage,File:HistoryFile,tools:{Fallback:PluginTool},data:{Fallback:({data})=><details className="tool-history"><summary>Событие провайдера</summary><ScrollArea asChild><pre>{JSON.stringify(data,null,2)}</pre></ScrollArea></details>},Reasoning:({text})=><details className="message-reasoning"><summary>Сводка рассуждений</summary><div>{text}</div></details>}}/>;
  return <MessagePrimitive.Root className={`message ${role}`} data-role={role}>
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

function Thread({ model, configured, providers, pluginModels, onSelect,remoteRunning,onRemoteCancel }: { model: string; configured: boolean; providers: Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void;remoteRunning?:boolean;onRemoteCancel?:()=>void }) {
  const scope=React.useContext(SessionScopeContext);
  const running=useAuiState(state=>state.thread.isRunning);
  const idle=useAuiState(state=>!state.composer.text.trim()&&!state.composer.attachments.length);
  const empty=useAuiState(state=>state.thread.isEmpty);
  return <ThreadPrimitive.Root className={empty?"thread thread-empty":"thread"}>
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
        <div className="harness-attachments"><ComposerPrimitive.Attachments>{({attachment})=><AttachmentPrimitive.Root className="harness-attachment"><AttachmentPrimitive.unstable_Thumb/><AttachmentPrimitive.Name/><AttachmentPrimitive.Remove asChild><Button type="button" variant="ghost" size="sm" aria-label={`Удалить ${attachment.name}`}>Удалить</Button></AttachmentPrimitive.Remove></AttachmentPrimitive.Root>}</ComposerPrimitive.Attachments></div><ScrollArea asChild><ComposerPrimitive.Input rows={1} disabled={!!remoteRunning} className="placeholder:oe-text-muted-foreground oe-min-h-8 oe-w-full oe-resize-none oe-text-foreground oe-border-0 oe-bg-transparent oe-px-3 oe-text-[15px] oe-outline-none focus-visible:oe-outline-none" placeholder="Напишите сообщение…" aria-label="Сообщение ассистенту" /></ScrollArea>
        <ComposerToolbar>
          <div className="composer-access-actions">
          <AttachmentPicker disabled={!!remoteRunning} renderMenu={onClose=><UiSlot slot="composer.menu" scope={workspaceScope} componentProps={{sessionId:scope.kind==='session'?scope.sessionId:'',disabled:!!remoteRunning,onClose}}/>}/>
          {scope.kind==='session'&&<UiSlot slot="composer.access" scope={workspaceScope} componentProps={{sessionId:scope.sessionId,disabled:!!remoteRunning||running}}/> }
          </div>
          <ComposerActions>
          <Select value={model || '__none'} onValueChange={value => onSelect(value === '__none' ? '' : value)}>
            <SelectTrigger asChild><ComposerModelTrigger model={model.replace(/^api:[^/]+\//, "") || "Выберите модель"} aria-label="Модель" /></SelectTrigger>
            <SelectContent align="end">
              <SelectItem value="__none">Выберите модель</SelectItem>
              {providers.map(provider => <SelectGroup key={provider.id}>
                <SelectLabel>{provider.name}</SelectLabel>
                {provider.models.map(name => <SelectItem key={name} value={`api:${provider.id}/${name}`}>{name}</SelectItem>)}
              </SelectGroup>)}
              {[...new Set(pluginModels.map(m=>m.group))].map(group=><SelectGroup key={`plugin:${group}`}><SelectLabel>{group}</SelectLabel>{pluginModels.filter(m=>m.group===group).map(m=><SelectItem key={m.id} value={m.id}>{m.name}</SelectItem>)}</SelectGroup>)}
            </SelectContent>
          </Select>
          <UiSlot slot="composer.actions" scope={scope} componentProps={{model,onSelect}}/><AuiIf condition={state=>(!state.thread.isRunning||!idle)&&!remoteRunning}><QueueComposerSend asChild><ComposerSend streaming={false} idle={idle} aria-label="Отправить сообщение"/></QueueComposerSend></AuiIf><AuiIf condition={state=>state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Cancel asChild><ComposerSend streaming idle={false} aria-label="Остановить ответ"/></ComposerPrimitive.Cancel></AuiIf><AuiIf condition={()=>!!remoteRunning}><ComposerSend streaming idle={false} aria-label="Остановить ответ" onClick={onRemoteCancel}/></AuiIf>
          </ComposerActions>
        </ComposerToolbar>
        </ComposerBar>
      </QueueComposerRoot>
      {!configured&&<p className="composer-hint">Модель не настроена: выберите её в списке или добавьте провайдера в разделе «Настройки → Модели».</p>}
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
  const adapter = useMemo<ChatModelAdapter>(() => {
    return chatAdapter({invoke,serverOwned:true,isAttached:()=>attached.current,sessionId:session.id,model,history,onRunning:setRunning,onSaved,onError,
      options:{}});
  }, [model, session.id, history]);
  const attachments=useMemo(()=>attachmentAdapter(session.id,invoke),[session.id]);
  const runtime = useLocalRuntime(adapter,{...messageQueueOptions,adapters:{history,attachments}});
  const remote=useRemoteSession(session.id);
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

    <UiSlot slot="session.controls" scope={workspaceScope} componentProps={{sessionId:session.id,running:running||!!remote.activeRun}}/><UiSlot slot="conversation.views" scope={sessionScope(session.id)} entryId="default" componentProps={{session,model,configured,providers,pluginModels,onSelect}} fallback={<Thread model={model} configured={configured} providers={providers} pluginModels={pluginModels} onSelect={onSelect} remoteRunning={!!remote.activeRun&&!localRun.current} onRemoteCancel={cancelRemote}/>}/><UiSlot slot="workspace.panels" scope={workspaceScope} componentProps={{session}}/>

  </AssistantRuntimeProvider></SessionScopeContext.Provider></SessionIdContext.Provider>;
}

export function Chat(props:React.ComponentProps<typeof SessionChat>){
 return <NavigationShell session={props.session} sessions={props.sessions} onSession={props.onSession} onNew={props.onNew} providers={props.providers} onProviders={props.onProviders} activePlugins={props.activePlugins}><SessionChat key={props.session.id} {...props}/></NavigationShell>;
}
