import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke } from '@tauri-apps/api/core';
import {
  AssistantRuntimeProvider, useLocalRuntime, ThreadPrimitive, AuiIf,
  MessagePrimitive, ComposerPrimitive, type ChatModelAdapter,
} from '@assistant-ui/react';
import { ArrowUp, MessageSquare, Plus, Puzzle, Sparkles } from 'lucide-react';
import { Button } from './components/ui/button';
import { Select, SelectGroup, SelectLabel, SelectTrigger, SelectValue, SelectContent, SelectItem } from './components/ui/select';
import './style.css';
import { Plugins } from './Plugins';
import { Settings, type Provider } from './Settings';
import { CodexModelsContext, type CodexModel, type CodexOptions } from './CodexSettings';
import { sessionHistory, type SessionInfo } from './sessionHistory';


function Message({ role }: { role: 'user' | 'assistant' }) {
  return <MessagePrimitive.Root className={`message ${role}`} data-role={role}>
    <div className="message-content"><div className="message-text"><MessagePrimitive.Parts components={{Reasoning:({text})=><details className="message-reasoning"><summary>Сводка рассуждений</summary><div>{text}</div></details>}}/></div></div>
  </MessagePrimitive.Root>;
}

function Thread({ model, configured, providers, codexModels, onSelect }: { model: string; configured: boolean; providers: Provider[]; codexModels:CodexModel[]; onSelect:(value:string)=>void }) {
  return <ThreadPrimitive.Root className="thread">
    <ThreadPrimitive.Viewport className="viewport">
      <AuiIf condition={(state) => state.thread.isEmpty}><div className="empty">
        <div className="empty-icon"><Sparkles size={20}/></div>
        <h1>С чего начнём?</h1>
        <p>Задайте вопрос или расскажите, над чем работаете.</p>
        <div className="empty-note"><span className={configured ? "status-dot" : "status-dot unavailable"}/> {!model ? 'Добавьте провайдера в настройках' : configured ? (model.startsWith('codex:') ? 'Codex подключён' : 'Ключ настроен') : 'Ключ не задан'} <span className="empty-divider"/> {model || 'Модель не выбрана'}</div>
      </div></AuiIf>
      <div className="messages"><ThreadPrimitive.Messages>{({ message }) => <Message role={message.role === "user" ? "user" : "assistant"}/>}</ThreadPrimitive.Messages></div>
    </ThreadPrimitive.Viewport>
    <div className="composer-wrap">
      <ComposerPrimitive.Root className="composer">
        <ComposerPrimitive.Input rows={1} className="composer-input" placeholder="Напишите сообщение…" aria-label="Сообщение ассистенту" />
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
              {codexModels.length > 0 && <SelectGroup>
                <SelectLabel>ChatGPT subscription</SelectLabel>
                {codexModels.map(model => <SelectItem key={model.id} value={`codex:${model.id}`}>{model.name}</SelectItem>)}
              </SelectGroup>}
            </SelectContent>
          </Select>
          <ComposerPrimitive.Send className="send" aria-label="Отправить сообщение"><ArrowUp size={16}/></ComposerPrimitive.Send>
        </div>
      </ComposerPrimitive.Root>
      <div className="composer-hint">Enter — отправить <span>·</span> Shift + Enter — новая строка</div>
    </div>
  </ThreadPrimitive.Root>;
}

function Chat({ model, configured, providers, codexModels, onSelect, onProviders, activePlugins, onNew, session, sessions, onSession, onSaved, onError }: { model: string; configured: boolean; providers:Provider[]; codexModels:CodexModel[]; onSelect:(value:string)=>void; onProviders:(p:Provider[])=>void; activePlugins:string[]; onNew:()=>void; session:SessionInfo; sessions:SessionInfo[]; onSession:(session:SessionInfo)=>void; onSaved:()=>void; onError:(message:string)=>void }) {
  const {options}=React.useContext(CodexModelsContext);
  const pendingIntegration=useRef<{namespace:string;value:unknown}|null>(null);
  const [running,setRunning]=useState(false);
  const history=useMemo(()=>sessionHistory(session.id,onSaved,onError,pendingIntegration),[session.id]);
  const adapter = useMemo<ChatModelAdapter>(() => ({
    async run({ messages }) {
      setRunning(true);
      try {
        const last=messages.at(-1);
        if(!last || last.role!=='user') throw new Error('Нужно сообщение пользователя.');
        await history.append({parentId:messages.at(-2)?.id??null,message:last});
        const selected=codexModels.find(item=>`codex:${item.id}`===model);
        if(model.startsWith('codex:') && options.effort && selected?.reasoningEfforts?.length && !selected.reasoningEfforts.includes(options.effort)) throw new Error('Выбранный уровень рассуждений не поддерживается этой моделью. Измените его в настройках Codex.');
        const result=await invoke<{text:string;summary:string|null;integration:{namespace:string;value:unknown}|null}>('session_run',{id:session.id,model,lastMessageId:last.id,options:{effort:options.effort||selected?.defaultReasoningEffort||null,summary:options.summary||'auto',verbosity:options.verbosity||null}});
        pendingIntegration.current=result.integration;
        if(result.summary)return {content:[{type:'reasoning' as const,text:result.summary},{type:'text' as const,text:result.text}]};
        return {content:[{type:'text' as const,text:result.text}]};
      } catch (error) {
        pendingIntegration.current=null;
        return { content: [{ type: 'text', text: `Не удалось получить ответ. ${String(error)}` }] };
      } finally { setRunning(false); }
    },
  }), [model, options, codexModels, session.id, history]);
  const runtime = useLocalRuntime(adapter,{adapters:{history}});
  const [view, setView] = useState<'chat' | 'plugins'>('chat');
  return <AssistantRuntimeProvider runtime={runtime}><div className="oe-studio app-shell">
    <aside className="sidebar">
      <div className="brand"><div className="brand-mark"><Sparkles size={18}/></div><div><strong>OE Studio</strong><small>AGENT WORKSPACE</small></div></div>
      <div className="sidebar-section"><div className="side-heading">РАБОЧЕЕ ПРОСТРАНСТВО</div><Button variant="outline" className="new-chat" disabled={running} onClick={() => { setView('chat'); onNew(); }}><Plus data-icon="inline-start"/> Новая сессия</Button></div>
      <div className="side-heading recent-heading">ДИАЛОГИ</div><div className="session-list" aria-label="История диалогов">{sessions.map(item=><button key={item.id} disabled={running} className={`nav-item ${view==='chat'&&item.id===session.id?'selected':''}`} title={item.title} onClick={()=>{setView('chat');onSession(item);}}><MessageSquare size={15}/><span>{item.title}</span></button>)}</div><div className="side-heading plugins-heading">РАСШИРЕНИЯ</div><button className={`nav-item ${view === "plugins" ? "selected" : ""}`} onClick={() => setView("plugins")}><Puzzle size={15}/><span>Плагины</span></button>
      <div className="sidebar-bottom"><Settings providers={providers} onChange={onProviders} activePlugins={activePlugins}/></div>
    </aside>
    <main className="main"><header className="topbar"><div className="breadcrumbs"><span>Чат</span><span className="crumb-separator">/</span><strong>{view === "chat" ? session.title : "Плагины"}</strong></div><div className="top-status"><span className="status-dot"/> Rig · локально</div></header>
      <div className="chat-view" hidden={view !== "chat"}><Thread model={model} configured={configured} providers={providers} codexModels={codexModels} onSelect={onSelect}/></div>
      {view === "plugins" && <Plugins/>}
    </main>
  </div></AssistantRuntimeProvider>;
}

function App() {
  const [sessions,setSessions]=useState<SessionInfo[]>([]);
  const [session,setSession]=useState<SessionInfo|null>(null);
  const [sessionError,setSessionError]=useState('');
  const [sessionLoading,setSessionLoading]=useState(true);
  const initialized=useRef(false);
  async function refreshSessions(){try{const list=await invoke<SessionInfo[]>('session_list');setSessions(list);setSession(current=>list.find(item=>item.id===current?.id)||current);}catch(error){setSessionError(`Не удалось получить историю: ${String(error)}`);}}
  async function createSession(){try{setSessionError('');const id=crypto.randomUUID();await invoke('session_create',{id});if(model)await invoke('session_set_model',{id,model});const item=await invoke<SessionInfo>('session_get',{id});setSession(item);setSessions(current=>[item,...current]);}catch(error){setSessionError(`Не удалось создать диалог: ${String(error)}`);}}
  useEffect(()=>{if(initialized.current)return;initialized.current=true;void (async()=>{try{const list=await invoke<SessionInfo[]>('session_list');setSessions(list);if(list.length){setSession(list[0]);setModel(list[0].model||'');}else await createSession();}catch(error){setSessionError(`Не удалось загрузить историю: ${String(error)}`);}finally{setSessionLoading(false);}})();},[]);
  const [providers,setProviders]=useState<Provider[]>([]);
  const [model,setModel]=useState('');
  const [codexModels,setCodexModels]=useState<CodexModel[]>([]);
  const [options,setOptions]=useState<CodexOptions>(()=>{try{const saved=JSON.parse(localStorage.getItem('codex.modelOptions')||'{}');return {effort:typeof saved.effort==='string'?saved.effort:'',summary:typeof saved.summary==='string'?saved.summary:'',verbosity:typeof saved.verbosity==='string'?saved.verbosity:''};}catch{return {effort:'',summary:'',verbosity:''};}});
  useEffect(()=>{localStorage.setItem('codex.modelOptions',JSON.stringify(options));},[options]);
  const [hidden,setHidden]=useState<string[]>(()=>{try{const v=JSON.parse(localStorage.getItem('codex.hiddenModels')||'[]');return Array.isArray(v)?v.filter(x=>typeof x==='string'):[];}catch{return [];}});
  const [defaultModel,setDefaultModel]=useState(()=>localStorage.getItem('codex.defaultModel')||'');
  useEffect(()=>{localStorage.setItem('codex.hiddenModels',JSON.stringify(hidden));},[hidden]);
  useEffect(()=>{localStorage.setItem('codex.defaultModel',defaultModel);},[defaultModel]);
  const [codexError,setCodexError]=useState('');
  const [codexLoading,setCodexLoading]=useState(false);
  async function refreshCodexModels(){setCodexLoading(true);setCodexError('');try{setCodexModels(await invoke<CodexModel[]>('codex_models'));}catch(e){setCodexError(String(e));}finally{setCodexLoading(false);}}
  const [activePlugins,setActivePlugins]=useState<string[]>([]);
  useEffect(()=>{ void invoke<Provider[]>('model_providers').then(setProviders).catch(()=>{}); void invoke<{id:string;enabled:boolean}[]>('list_plugins').then(items=>setActivePlugins(items.filter(x=>x.enabled).map(x=>x.id))).catch(()=>{}); },[]);
  useEffect(()=>{if(activePlugins.includes('codex'))void refreshCodexModels();else setCodexModels([]);},[activePlugins]);
  useEffect(()=>{if(!model && codexModels.length){const preferred=codexModels.find(m=>m.id===defaultModel&&!hidden.includes(m.id))||codexModels.find(m=>m.isDefault&&!hidden.includes(m.id));if(preferred){const value=`codex:${preferred.id}`;setModel(value);if(session)void invoke('session_set_model',{id:session.id,model:value}).catch(error=>setSessionError(`Не удалось сохранить модель: ${String(error)}`));}}},[codexModels,defaultModel,hidden,model,session?.id]);
  const configured=model.startsWith('codex:')?codexModels.some(m=>`codex:${m.id}`===model):providers.some(p=>p.hasKey && p.models.some(m=>`api:${p.id}/${m}`===model));
  function selectSession(item:SessionInfo){setSessionError('');setSession(item);setModel(item.model||'');}
  function selectModel(value:string){setModel(value);if(session){const id=session.id;setSessions(current=>current.map(item=>item.id===id?{...item,model:value}:item));setSession(current=>current?.id===id?{...current,model:value}:current);void invoke('session_set_model',{id,model:value}).catch(error=>setSessionError(`Не удалось сохранить модель: ${String(error)}`));}}
  if(!session)return <div className="oe-studio session-startup" role="status">{sessionLoading?'Загружаем историю…':sessionError||'Нет доступных диалогов'}{!sessionLoading&&<Button variant="outline" onClick={()=>void createSession()}>Повторить</Button>}</div>;
  return <CodexModelsContext.Provider value={{models:codexModels,loading:codexLoading,error:codexError,refresh:refreshCodexModels,hidden,setHidden,defaultModel,setDefaultModel,options,setOptions}}>{sessionError&&<div className="session-error" role="alert">{sessionError}<button aria-label="Закрыть уведомление" onClick={()=>setSessionError('')}>×</button></div>}<Chat key={session.id} session={session} sessions={sessions} onSession={selectSession} onSaved={()=>void refreshSessions()} onError={setSessionError} model={model} configured={configured} providers={providers} codexModels={codexModels.filter(m=>!hidden.includes(m.id)||model===`codex:${m.id}`)} onSelect={selectModel} onProviders={setProviders} activePlugins={activePlugins} onNew={()=>void createSession()}/></CodexModelsContext.Provider>;
}

createRoot(document.getElementById('root')!).render(<React.StrictMode><App/></React.StrictMode>);
