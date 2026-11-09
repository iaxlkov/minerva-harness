import {NavigationShell} from '../../app/NavigationShell';
import { ScrollArea } from '../../components/ui/scroll-area';
import {AttachmentPicker} from '../../harness/AttachmentPicker';
import {SessionControls} from '../../harness/SessionControls';
import {SessionFile} from '../../harness/SessionFile';
import {SessionImage} from '../../harness/SessionImage';
import {attachmentAdapter} from '../../harness/attachments';
import {AttachmentPrimitive} from '@assistant-ui/react';
import React, { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../../client.ts';
import {
  AssistantRuntimeProvider, useLocalRuntime, ThreadPrimitive, AuiIf,
  MessagePrimitive, ComposerPrimitive, type ChatModelAdapter,
} from '@assistant-ui/react';
import { ArrowUp } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Select, SelectGroup, SelectLabel, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../components/ui/select';
import '../../style.css';
import type { Provider, SessionInfo, PluginModel } from '../../../../../packages/client/src/types.ts';
import { sessionHistory,decodeSessionMessage } from '../../sessionHistory';
import {useRemoteSession} from '../../app/useRemoteSession';
import { chatAdapter } from '../../chatAdapter';
import { ToolHistory } from '../../ToolHistory';

import {UiSlot,workspaceScope,sessionScope} from "../../app/plugins";
const SessionIdContext=React.createContext('');
const SessionScopeContext=React.createContext(sessionScope("unselected"));
function PluginTool(props:React.ComponentProps<typeof ToolHistory>){const scope=React.useContext(SessionScopeContext);return <UiSlot slot="tool.renderers" scope={scope} entryId={props.toolName} componentProps={{...props}} fallback={<ToolHistory {...props}/>}/>;}
function HistoryFile(props:Omit<React.ComponentProps<typeof SessionFile>,'sessionId'>){return <SessionFile {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function HistoryImage(props:Omit<React.ComponentProps<typeof SessionImage>,'sessionId'>){return <SessionImage {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function Message({ role }: { role: 'user' | 'assistant' }) {
  const scope=React.useContext(SessionScopeContext);
  const standard=<MessagePrimitive.Parts components={{Image:HistoryImage,File:HistoryFile,tools:{Fallback:PluginTool},data:{Fallback:({data})=><details className="tool-history"><summary>Событие провайдера</summary><ScrollArea asChild><pre>{JSON.stringify(data,null,2)}</pre></ScrollArea></details>},Reasoning:({text})=><details className="message-reasoning"><summary>Сводка рассуждений</summary><div>{text}</div></details>}}/>;
  return <MessagePrimitive.Root className={`message ${role}`} data-role={role}>
    <div className="message-content"><div className="message-text"><UiSlot slot="message.renderers" scope={scope} entryId={role} componentProps={{children:standard}} fallback={standard}/><AuiIf condition={s=>s.message.status?.type==='incomplete'}><div className="message-interrupted">Ход не завершён. Сохранённые действия показаны выше.</div></AuiIf></div></div>
  </MessagePrimitive.Root>;
}

function Thread({ model, configured, providers, pluginModels, onSelect,remoteRunning,onRemoteCancel }: { model: string; configured: boolean; providers: Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void;remoteRunning?:boolean;onRemoteCancel?:()=>void }) {
  const scope=React.useContext(SessionScopeContext);
  return <ThreadPrimitive.Root className="thread">
    <ScrollArea asChild><ThreadPrimitive.Viewport className="viewport">
      <AuiIf condition={(state) => state.thread.isEmpty}><div className="empty">
        <img className="empty-portrait" src="/minerva-welcome.png" width={280} height={280} alt="Минерва — ваш ассистент"/>
        <h1>С чего начнём?</h1>
        <p>Задайте вопрос или расскажите, над чем работаете.</p>
      </div></AuiIf>
      <div className="messages"><ThreadPrimitive.Messages>{({ message }) => <Message role={message.role === "user" ? "user" : "assistant"}/>}</ThreadPrimitive.Messages></div>
    </ThreadPrimitive.Viewport></ScrollArea>
    <div className="composer-wrap">
      <ComposerPrimitive.Root className="composer">
        <div className="harness-attachments"><ComposerPrimitive.Attachments>{({attachment})=><AttachmentPrimitive.Root className="harness-attachment"><AttachmentPrimitive.unstable_Thumb/><AttachmentPrimitive.Name/><AttachmentPrimitive.Remove asChild><Button type="button" variant="ghost" size="sm" aria-label={`Удалить ${attachment.name}`}>Удалить</Button></AttachmentPrimitive.Remove></AttachmentPrimitive.Root>}</ComposerPrimitive.Attachments></div><ScrollArea asChild><ComposerPrimitive.Input rows={1} disabled={!!remoteRunning} className="composer-input" placeholder="Напишите сообщение…" aria-label="Сообщение ассистенту" /></ScrollArea>
        <div className="composer-bottom">
          <AttachmentPicker disabled={!!remoteRunning} renderMenu={onClose=><UiSlot slot="composer.menu" scope={workspaceScope} componentProps={{sessionId:scope.kind==='session'?scope.sessionId:'',disabled:!!remoteRunning,onClose}}/>}/>
          <Select value={model || '__none'} onValueChange={value => onSelect(value === '__none' ? '' : value)}>
            <SelectTrigger className="composer-select" aria-label="Модель">
              <span className={configured ? "status-dot" : "status-dot unavailable"} aria-hidden="true"/>
              <SelectValue/>
            </SelectTrigger>
            <SelectContent align="end">
              <SelectItem value="__none">Выберите модель</SelectItem>
              {providers.map(provider => <SelectGroup key={provider.id}>
                <SelectLabel>{provider.name}</SelectLabel>
                {provider.models.map(name => <SelectItem key={name} value={`api:${provider.id}/${name}`}>{name}</SelectItem>)}
              </SelectGroup>)}
              {[...new Set(pluginModels.map(m=>m.group))].map(group=><SelectGroup key={`plugin:${group}`}><SelectLabel>{group}</SelectLabel>{pluginModels.filter(m=>m.group===group).map(m=><SelectItem key={m.id} value={m.id}>{m.name}</SelectItem>)}</SelectGroup>)}
            </SelectContent>
          </Select>
          <UiSlot slot="composer.actions" scope={scope} componentProps={{model,onSelect}}/><AuiIf condition={state=>!state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Send asChild><Button size="icon" className="send" aria-label="Отправить сообщение"><ArrowUp size={16}/></Button></ComposerPrimitive.Send></AuiIf><AuiIf condition={state=>state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Cancel asChild><Button size="icon" className="send" aria-label="Остановить ответ">■</Button></ComposerPrimitive.Cancel></AuiIf><AuiIf condition={()=>!!remoteRunning}><Button variant="ghost" className="send" aria-label="Остановить ответ" onClick={onRemoteCancel}>■</Button></AuiIf>
        </div>
      </ComposerPrimitive.Root>
      {remoteRunning&&<AuiIf condition={state=>!state.thread.isRunning}><div role="status">Задача выполняется на сервере…</div></AuiIf>}
      <div className="composer-hint">Enter — отправить <span>·</span> Shift + Enter — новая строка</div>
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
  const runtime = useLocalRuntime(adapter,{adapters:{history,attachments}});
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

    <SessionControls key={session.id} sessionId={session.id} running={running||!!remote.activeRun}/><UiSlot slot="conversation.views" scope={sessionScope(session.id)} entryId="default" componentProps={{session,model,configured,providers,pluginModels,onSelect}} fallback={<Thread model={model} configured={configured} providers={providers} pluginModels={pluginModels} onSelect={onSelect} remoteRunning={!!remote.activeRun} onRemoteCancel={cancelRemote}/>}/><UiSlot slot="workspace.panels" scope={workspaceScope} componentProps={{session}}/>

  </AssistantRuntimeProvider></SessionScopeContext.Provider></SessionIdContext.Provider>;
}

export function Chat(props:React.ComponentProps<typeof SessionChat>){
 return <NavigationShell session={props.session} sessions={props.sessions} onSession={props.onSession} onNew={props.onNew} providers={props.providers} onProviders={props.onProviders} activePlugins={props.activePlugins}><SessionChat key={props.session.id} {...props}/></NavigationShell>;
}
