import {AttachmentPicker} from '../../harness/AttachmentPicker';
import {SessionControls} from '../../harness/SessionControls';
import {SessionImage} from '../../harness/SessionImage';
import {attachmentAdapter} from '../../harness/attachments';
import {AttachmentPrimitive} from '@assistant-ui/react';
import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../../client.ts';
import {
  AssistantRuntimeProvider, useLocalRuntime, ThreadPrimitive, AuiIf,
  MessagePrimitive, ComposerPrimitive, type ChatModelAdapter,
} from '@assistant-ui/react';
import { ArrowUp, MessageSquare, Plus, Puzzle } from 'lucide-react';
import { Button } from '../../components/ui/button';
import { Select, SelectGroup, SelectLabel, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../../components/ui/select';
import '../../style.css';
import { Plugins } from '../../Plugins';
import { Settings } from '../../Settings';
import type { Provider, SessionInfo, PluginModel } from '../../../../../packages/client/src/types.ts';
import { sessionHistory,decodeSessionMessage } from '../../sessionHistory';
import {useRemoteSession} from '../../app/useRemoteSession';
import { chatAdapter } from '../../chatAdapter';
import { ToolHistory } from '../../ToolHistory';

import {UiSlot,workspaceScope,sessionScope} from "../../app/plugins";
const SessionIdContext=React.createContext('');
const SessionScopeContext=React.createContext(sessionScope("unselected"));
function PluginTool(props:React.ComponentProps<typeof ToolHistory>){const scope=React.useContext(SessionScopeContext);return <UiSlot slot="tool.renderers" scope={scope} entryId={props.toolName} componentProps={{...props}} fallback={<ToolHistory {...props}/>}/>;}
function HistoryImage(props:Omit<React.ComponentProps<typeof SessionImage>,'sessionId'>){return <SessionImage {...props} sessionId={React.useContext(SessionIdContext)}/>;}
function Message({ role }: { role: 'user' | 'assistant' }) {
  const scope=React.useContext(SessionScopeContext);
  const standard=<MessagePrimitive.Parts components={{Image:HistoryImage,tools:{Fallback:PluginTool},data:{Fallback:({data})=><details className="tool-history"><summary>Событие провайдера</summary><pre>{JSON.stringify(data,null,2)}</pre></details>},Reasoning:({text})=><details className="message-reasoning"><summary>Сводка рассуждений</summary><div>{text}</div></details>}}/>;
  return <MessagePrimitive.Root className={`message ${role}`} data-role={role}>
    <div className="message-content"><div className="message-text"><UiSlot slot="message.renderers" scope={scope} entryId={role} componentProps={{children:standard}} fallback={standard}/><AuiIf condition={s=>s.message.status?.type==='incomplete'}><div className="message-interrupted">Ход не завершён. Сохранённые действия показаны выше.</div></AuiIf></div></div>
  </MessagePrimitive.Root>;
}

function Thread({ model, configured, providers, pluginModels, onSelect,remoteRunning,onRemoteCancel }: { model: string; configured: boolean; providers: Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void;remoteRunning?:boolean;onRemoteCancel?:()=>void }) {
  const scope=React.useContext(SessionScopeContext);
  return <ThreadPrimitive.Root className="thread">
    <ThreadPrimitive.Viewport className="viewport">
      <AuiIf condition={(state) => state.thread.isEmpty}><div className="empty">
        <img className="empty-portrait" src="/minerva-welcome.png" width={280} height={280} alt="Минерва — ваш ассистент"/>
        <h1>С чего начнём?</h1>
        <p>Задайте вопрос или расскажите, над чем работаете.</p>
      </div></AuiIf>
      <div className="messages"><ThreadPrimitive.Messages>{({ message }) => <Message role={message.role === "user" ? "user" : "assistant"}/>}</ThreadPrimitive.Messages></div>
    </ThreadPrimitive.Viewport>
    <div className="composer-wrap">
      <ComposerPrimitive.Root className="composer">
        <div className="harness-attachments"><ComposerPrimitive.Attachments>{({attachment})=><AttachmentPrimitive.Root className="harness-attachment"><AttachmentPrimitive.unstable_Thumb/><AttachmentPrimitive.Name/><AttachmentPrimitive.Remove aria-label={`Удалить ${attachment.name}`}>Удалить</AttachmentPrimitive.Remove></AttachmentPrimitive.Root>}</ComposerPrimitive.Attachments></div><ComposerPrimitive.Input rows={1} disabled={!!remoteRunning} className="composer-input" placeholder="Напишите сообщение…" aria-label="Сообщение ассистенту" />
        <div className="composer-bottom">
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
          <AttachmentPicker disabled={!!remoteRunning}/><UiSlot slot="composer.actions" scope={scope} componentProps={{model,onSelect}}/><AuiIf condition={state=>!state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Send className="send" aria-label="Отправить сообщение"><ArrowUp size={16}/></ComposerPrimitive.Send></AuiIf><AuiIf condition={state=>state.thread.isRunning&&!remoteRunning}><ComposerPrimitive.Cancel className="send" aria-label="Остановить ответ">■</ComposerPrimitive.Cancel></AuiIf><AuiIf condition={()=>!!remoteRunning}><button className="send" aria-label="Остановить ответ" onClick={onRemoteCancel}>■</button></AuiIf>
        </div>
      </ComposerPrimitive.Root>
      {remoteRunning&&<AuiIf condition={state=>!state.thread.isRunning}><div role="status">Задача выполняется на сервере…</div></AuiIf>}
      <div className="composer-hint">Enter — отправить <span>·</span> Shift + Enter — новая строка</div>
    </div>
  </ThreadPrimitive.Root>;
}

import {models} from "../../app/models";

export function Chat({ model, configured, providers, pluginModels, onSelect, onProviders, activePlugins, onNew, session, sessions, onSession, onSaved, onError }: { model: string; configured: boolean; providers:Provider[]; pluginModels:PluginModel[];  onSelect:(value:string)=>void; onProviders:(p:Provider[])=>void; activePlugins:string[]; onNew:()=>void; session:SessionInfo; sessions:SessionInfo[]; onSession:(session:SessionInfo)=>void; onSaved:()=>void; onError:(message:string)=>void }) {
  const run=React.useSyncExternalStore(models.runs.subscribe,models.runs.getSnapshot);
  const running=run.running;
  const localRun=useRef(false);
  const setRunning=(running:boolean)=>{localRun.current=running;models.runs.baseline({...models.runs.getSnapshot(),running});};
  const history=useMemo(()=>sessionHistory(session.id,onSaved,onError,invoke,{serverOwned:true}),[session.id]);
  const adapter = useMemo<ChatModelAdapter>(() => {
    return chatAdapter({invoke,serverOwned:true,sessionId:session.id,model,history,onRunning:setRunning,onSaved,onError,
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
      models.runs.baseline({runId:remote.activeRun?.runId??null,parts:remote.activeRun?.parts??[],running:!!remote.activeRun});
    }
  },[remote,runtime,running]);
  const cancelRemote=()=>{if(remote.activeRun)void invoke('runs.cancel',{sessionId:session.id,runId:remote.activeRun.runId}).catch(error=>onError(`Не удалось отменить задачу: ${String(error)}`));};
  const [view, setView] = useState<'chat' | 'plugins'>('chat');
  return <SessionIdContext.Provider value={session.id}><SessionScopeContext.Provider value={sessionScope(session.id)}><AssistantRuntimeProvider runtime={runtime}><div className="oe-studio app-shell">
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark"><img src="/minerva-64.png" width={32} height={32} alt="" style={{borderRadius:9}}/></div><div><strong>Minerva Harness</strong><small>AGENT WORKSPACE</small></div></div>
      <div className="sidebar-section"><div className="side-heading">РАБОЧЕЕ ПРОСТРАНСТВО</div><Button variant="outline" className="new-chat" disabled={running} onClick={() => { setView('chat'); onNew(); }}><Plus data-icon="inline-start"/> Новая сессия</Button></div>
      <div className="side-heading recent-heading">ДИАЛОГИ</div><div className="session-list" aria-label="История диалогов">{sessions.map(item=><button key={item.id} disabled={running} className={`nav-item ${view==='chat'&&item.id===session.id?'selected':''}`} title={item.title} onClick={()=>{setView('chat');onSession(item);}}><MessageSquare size={15}/><span>{item.title}</span></button>)}</div><div className="side-heading plugins-heading">РАСШИРЕНИЯ</div><button className={`nav-item ${view === "plugins" ? "selected" : ""}`} onClick={() => setView("plugins")}><Puzzle size={15}/><span>Плагины</span></button>
      <UiSlot slot="navigation.items" scope={workspaceScope} componentProps={{session,onNew,onSession}}/><div className="sidebar-bottom"><Settings sessionId={session.id} providers={providers} onChange={onProviders} activePlugins={activePlugins}/></div>
    </aside>
    <main className="main"><header className="topbar"><div className="breadcrumbs"><span>Чат</span><span className="crumb-separator">/</span><strong>{view === "chat" ? session.title : "Плагины"}</strong></div><div className="top-status"><span className="status-dot"/> Rig · локально</div></header>
      <div className="chat-view" hidden={view !== "chat"}><SessionControls key={session.id} sessionId={session.id} running={running||!!remote.activeRun}/><UiSlot slot="conversation.views" scope={sessionScope(session.id)} entryId="default" componentProps={{session,model,configured,providers,pluginModels,onSelect}} fallback={<Thread model={model} configured={configured} providers={providers} pluginModels={pluginModels} onSelect={onSelect} remoteRunning={!!remote.activeRun} onRemoteCancel={cancelRemote}/>}/></div>
      <UiSlot slot="workspace.panels" scope={workspaceScope} componentProps={{session}}/>
      {view === "plugins" && <Plugins/>}
    </main>
  </div></AssistantRuntimeProvider></SessionScopeContext.Provider></SessionIdContext.Provider>;
}
