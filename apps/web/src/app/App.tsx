import React, { useEffect, useMemo, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { invoke, initializeClient } from '../client.ts';
import {
  AssistantRuntimeProvider, useLocalRuntime, ThreadPrimitive, AuiIf,
  MessagePrimitive, ComposerPrimitive, type ChatModelAdapter,
} from '@assistant-ui/react';
import { ArrowUp, MessageSquare, Plus, Puzzle } from 'lucide-react';
import { Button } from '../components/ui/button';
import { Select, SelectGroup, SelectLabel, SelectTrigger, SelectValue, SelectContent, SelectItem } from '../components/ui/select';
import '../style.css';
import { Plugins } from '../Plugins';
import { Settings } from '../Settings';
import type { Provider, CodexModel, CodexOptions, SessionInfo } from '../../../../packages/client/src/types.ts';
import { CodexModelsContext } from '../CodexSettings';
import { sessionHistory } from '../sessionHistory';
import { chatAdapter } from '../chatAdapter';
import { ToolHistory } from '../ToolHistory';

import { Chat } from "../pages/chat/Chat";
import { models, preferences } from "./models";

import {useUiPlugins,reconcileBuiltinUi} from "./plugins";
export function App() {
  const uiPlugins=useUiPlugins();
  const sessions=React.useSyncExternalStore(models.sessions.subscribe,models.sessions.getSnapshot);
  const setSessions=(next:SessionInfo[]|((current:SessionInfo[])=>SessionInfo[]))=>models.sessions.set(typeof next==='function'?next(models.sessions.getSnapshot()):next);
  const [session,setSession]=useState<SessionInfo|null>(null);
  const [sessionError,setSessionError]=useState('');
  const [sessionLoading,setSessionLoading]=useState(true);
  const initialized=useRef(false);
  async function refreshSessions(){try{await models.sessions.refresh();const list=models.sessions.getSnapshot();setSession(current=>list.find(item=>item.id===current?.id)||current);}catch(error){setSessionError(`Не удалось получить историю: ${String(error)}`);}}
  async function createSession(){try{setSessionError('');const item=await models.sessions.create(model);setSession(item);}catch(error){setSessionError(`Не удалось создать диалог: ${String(error)}`);}}
  useEffect(()=>{if(initialized.current)return;initialized.current=true;void (async()=>{try{await models.sessions.refresh();const list=models.sessions.getSnapshot();if(list.length){setSession(list[0]);setModel(list[0].model||'');}else await createSession();}catch(error){setSessionError(`Не удалось загрузить историю: ${String(error)}`);}finally{setSessionLoading(false);}})();},[]);
  const providers=React.useSyncExternalStore(models.providers.subscribe,models.providers.getSnapshot);
  const setProviders=models.providers.set;
  const [model,setModel]=useState('');
  useEffect(()=>{const latest=sessions.find(item=>item.id===session?.id);if(latest){setSession(latest);setModel(latest.model??'');}},[sessions,session?.id]);
  const codexModels=React.useSyncExternalStore(models.providers.codex.subscribe,models.providers.codex.getSnapshot);
  const setCodexModels=models.providers.codex.set;
  const [options,setOptions]=useState<CodexOptions>(()=>{try{const saved=JSON.parse(JSON.stringify(preferences().get('codex.modelOptions',{}))||'{}');return {effort:typeof saved.effort==='string'?saved.effort:'',summary:typeof saved.summary==='string'?saved.summary:'',verbosity:typeof saved.verbosity==='string'?saved.verbosity:''};}catch{return {effort:'',summary:'',verbosity:''};}});
  useEffect(()=>{preferences().set('codex.modelOptions',options);},[options]);
  const [hidden,setHidden]=useState<string[]>(()=>{try{const v=JSON.parse(JSON.stringify(preferences().get('codex.hiddenModels',[]))||'[]');return Array.isArray(v)?v.filter(x=>typeof x==='string'):[];}catch{return [];}});
  const [defaultModel,setDefaultModel]=useState(()=>preferences().get('codex.defaultModel','')||'');
  useEffect(()=>{preferences().set('codex.hiddenModels',hidden);},[hidden]);
  useEffect(()=>{preferences().set('codex.defaultModel',defaultModel);},[defaultModel]);
  const [codexError,setCodexError]=useState('');
  const [codexLoading,setCodexLoading]=useState(false);
  async function refreshCodexModels(){setCodexLoading(true);setCodexError('');try{await models.providers.refreshCodex();}catch(e){setCodexError(String(e));}finally{setCodexLoading(false);}}
  const plugins=React.useSyncExternalStore(models.plugins.subscribe,models.plugins.getSnapshot);
  const activePlugins=useMemo(()=>plugins.filter(x=>x.enabled).map(x=>x.id),[plugins]);
  useEffect(()=>{void reconcileBuiltinUi(uiPlugins,activePlugins).catch(error=>setSessionError(String(error)));},[uiPlugins,activePlugins]);
  useEffect(()=>{ void models.providers.refresh().catch(error=>setSessionError(`Не удалось загрузить провайдеров: ${String(error)}`)); void models.plugins.refresh().catch(error=>setSessionError(`Не удалось загрузить модули: ${String(error)}`)); },[]);
  useEffect(()=>{if(activePlugins.includes('codex'))void refreshCodexModels();else setCodexModels([]);},[activePlugins]);
  useEffect(()=>{if(!model && codexModels.length){const preferred=codexModels.find(m=>m.id===defaultModel&&!hidden.includes(m.id))||codexModels.find(m=>m.isDefault&&!hidden.includes(m.id));if(preferred&&session){selectModel(`codex:${preferred.id}`);}}},[codexModels,defaultModel,hidden,model,session?.id]);
  const configured=model.startsWith('codex:')?codexModels.some(m=>`codex:${m.id}`===model):providers.some(p=>p.hasKey && p.models.some(m=>`api:${p.id}/${m}`===model));
  function selectSession(item:SessionInfo){setSessionError('');setSession(item);setModel(item.model||'');}
  const modelWrites=useRef(Promise.resolve());
  const activeSessionId=useRef(session?.id);activeSessionId.current=session?.id;
  function selectModel(value:string){
    if(!session)return;
    const id=session.id;
    modelWrites.current=modelWrites.current.then(async()=>{
      try {
        await invoke('session_set_model',{id,model:value});
        setSessions(current=>current.map(item=>item.id===id?{...item,model:value}:item));
        if(activeSessionId.current===id){setModel(value);setSession(current=>current?.id===id?{...current,model:value}:current);}
      }catch(error){setSessionError(`Не удалось сохранить модель: ${String(error)}`);}
    });
  }

  if(!session)return <div className="oe-studio session-startup" role="status">{sessionLoading?'Загружаем историю…':sessionError||'Нет доступных диалогов'}{!sessionLoading&&<Button variant="outline" onClick={()=>void createSession()}>Повторить</Button>}</div>;
  return <CodexModelsContext.Provider value={{models:codexModels,loading:codexLoading,error:codexError,refresh:refreshCodexModels,hidden,setHidden,defaultModel,setDefaultModel,options,setOptions}}>{sessionError&&<div className="session-error" role="alert">{sessionError}<button aria-label="Закрыть уведомление" onClick={()=>setSessionError('')}>×</button></div>}<Chat key={session.id} session={session} sessions={sessions} onSession={selectSession} onSaved={()=>void refreshSessions()} onError={setSessionError} model={model} configured={configured} providers={providers} codexModels={codexModels.filter(m=>!hidden.includes(m.id)||model===`codex:${m.id}`)} onSelect={selectModel} onProviders={setProviders} activePlugins={activePlugins} onNew={()=>void createSession()}/></CodexModelsContext.Provider>;
}
