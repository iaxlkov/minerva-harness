import React, { useEffect, useMemo, useRef, useState } from 'react';
import { invoke } from '../client.ts';
import { Button } from '../components/ui/button';
import { Loader2, X } from 'lucide-react';
import { TitleBar } from './TitleBar';
import type { SessionInfo, PluginModel } from '../../../../packages/client/src/types.ts';
import { Chat } from '../pages/chat/Chat';
import { models } from './models';
import '../style.css';

export function App() {
  const sessions=React.useSyncExternalStore(models.sessions.subscribe,models.sessions.getSnapshot);
  const setSessions=(next:SessionInfo[]|((current:SessionInfo[])=>SessionInfo[]))=>models.sessions.set(typeof next==='function'?next(models.sessions.getSnapshot()):next);
  const [session,setSession]=useState<SessionInfo|null>(null);
  const [sessionError,setSessionError]=useState('');
  const [sessionLoading,setSessionLoading]=useState(true);
  const initialized=useRef(false);
  const creatingSession=useRef(false);
  async function refreshSessions(){try{await models.sessions.refresh();const list=models.sessions.getSnapshot();setSession(current=>list.find(item=>item.id===current?.id)||current);}catch(error){setSessionError(`Не удалось получить историю: ${String(error)}`);}}
  async function createSession(workspaceId?:string){
    if(creatingSession.current)return;
    creatingSession.current=true;
    try{
      setSessionError('');
      const workspaces=session?await invoke('workspaces.list'):[];
      const currentWorkspace=workspaces.find(item=>item.sessionIds.includes(session?.id??''))?.id;
      if(!workspaceId&&session&&!currentWorkspace)throw Error('Не удалось определить workspace текущего диалога.');
      workspaceId??=currentWorkspace??'default';
      const target=workspaces.find(item=>item.id===workspaceId);
      const archived=new Set(target?.archivedSessionIds??[]);
      const draftId=workspaceId===currentWorkspace&&!archived.has(session?.id??'')?session?.id:models.sessions.getSnapshot().find(item=>item.hasMessages===false&&!archived.has(item.id)&&target?.sessionIds.includes(item.id))?.id;
      const item=await models.sessions.create(model,workspaceId,draftId);setSession(item);
    }catch(error){setSessionError(`Не удалось создать диалог: ${String(error)}`);}finally{creatingSession.current=false;}
  }
  useEffect(()=>{if(initialized.current)return;initialized.current=true;void (async()=>{try{await models.sessions.refresh();await models.workspaces.refresh();const visible=new Set(models.workspaces.getSnapshot().flatMap(item=>item.sessionIds.filter(id=>!(item.archivedSessionIds??[]).includes(id))));const list=models.sessions.getSnapshot().filter(item=>visible.has(item.id));if(list.length){const initial=list.find(item=>item.hasMessages!==false)??list[0];setSession(initial);setModel(initial.model||'');}else await createSession();}catch(error){setSessionError(`Не удалось загрузить историю: ${String(error)}`);}finally{setSessionLoading(false);}})();},[]);
  const providers=React.useSyncExternalStore(models.providers.subscribe,models.providers.getSnapshot);
  const setProviders=models.providers.set;
  const [model,setModel]=useState('');
  useEffect(()=>{const latest=sessions.find(item=>item.id===session?.id);if(latest){setSession(latest);setModel(latest.model??'');}},[sessions,session?.id]);
  const plugins=React.useSyncExternalStore(models.plugins.subscribe,models.plugins.getSnapshot);
  const activePlugins=useMemo(()=>plugins.filter(x=>x.enabled).map(x=>x.id),[plugins]);
  useEffect(()=>{
    void models.providers.refresh().catch(error=>setSessionError(`Не удалось загрузить провайдеров: ${String(error)}`));
    let lastPluginError='';
    const unsubscribe=models.plugins.errors.subscribe(()=>{
      const error=models.plugins.errors.getSnapshot();
      const next=error?`Не удалось обновить модули: ${error}`:'';
      const previous=lastPluginError;lastPluginError=next;
      setSessionError(current=>next||(current===previous?'':current));
    });
    const stop=models.plugins.startPolling();
    return ()=>{stop();unsubscribe();};
  },[]);
  const [pluginModels,setPluginModels]=useState<PluginModel[]>([]);
  const enabledPackages=plugins.filter(p=>p.enabled&&p.origin==='installed').map(p=>p.id).sort().join('|');
  useEffect(()=>{
    let active=true;
    let timer:ReturnType<typeof setTimeout>;
    setPluginModels([]);
    if(!enabledPackages)return;
    async function refresh(){
      try{const catalog=await invoke('plugins.models');if(active)setPluginModels(catalog);}
      catch(error){if(active)setSessionError(String(error));}
      finally{if(active)timer=setTimeout(()=>void refresh(),3000);}
    }
    void refresh();
    return ()=>{active=false;clearTimeout(timer);};
  },[enabledPackages]);
  const availablePluginModels=pluginModels.filter(m=>plugins.some(p=>p.enabled&&m.id.startsWith(`plugin:${p.id}/`)));
  const configured=availablePluginModels.some(m=>m.id===model)||providers.some(p=>(p.hasKey||p.protocol==='ollama') && p.models.some(m=>`api:${p.id}/${m}`===model));
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

  const defaultAttempt=useRef('');
  useEffect(()=>{
    if(!session||model)return;
    const preferred=availablePluginModels.find(m=>m.isDefault)??availablePluginModels[0];
    const attempt=`${session.id}/${preferred?.id??''}`;
    if(!preferred||defaultAttempt.current===attempt)return;
    defaultAttempt.current=attempt;
    selectModel(preferred.id);
  },[session?.id,model,availablePluginModels]);

  if(!session)return <div className="oe-studio session-startup"><TitleBar title={<strong>Minerva</strong>}/><div className="session-startup-body" role="status">{sessionLoading&&<Loader2 className="oe-animate-spin" size={18} aria-hidden="true"/>}{sessionLoading?'Загружаем историю…':sessionError||'Нет доступных диалогов'}{!sessionLoading&&<Button variant="outline" onClick={()=>void createSession()}>Повторить</Button>}</div></div>;
  return <>{sessionError&&<div className="session-error" role="alert">{sessionError}<Button variant="ghost" size="icon-sm" aria-label="Закрыть уведомление" onClick={()=>setSessionError('')}><X/></Button></div>}<Chat session={session} sessions={sessions} onSession={selectSession} onSaved={()=>void refreshSessions()} onError={setSessionError} model={model} configured={configured} providers={providers} pluginModels={availablePluginModels} onSelect={selectModel} onProviders={setProviders} activePlugins={activePlugins} onNew={workspaceId=>void createSession(workspaceId)}/></>;
}
