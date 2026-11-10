import React, {useCallback, useEffect, useState} from 'react';
import {invoke} from '../../client';
import type {JsonValue, SessionInfo} from '../../../../../packages/client/src/types';
import {Button} from '../../components/ui/button';
import './learning.css';
import {CurationForms} from './CurationForms';
import {ScrollArea} from '../../components/ui/scroll-area';

type Row = Record<string, JsonValue>;
const labels: Record<string,string> = {'memory':'Память','session-query':'История','skills':'Навыки','experience':'Опыт','experiments':'Эксперименты','improvement':'Улучшение','skill-curation':'Предложения навыков'};
const listMethods: Record<string,string> = {'memory':'search','session-query':'search','skills':'list','experience':'search','experiments':'list','improvement':'list','skill-curation':'list'};
const readMethods: Record<string,string> = {'memory':'read','session-query':'read','skills':'load','experience':'get','experiments':'get','improvement':'get','skill-curation':'get'};
function records(value: JsonValue): Row[] {
  if (Array.isArray(value)) return value.filter((v): v is Row => !!v && typeof v === 'object' && !Array.isArray(v));
  if (value && typeof value === 'object') { const object = value as Row; for (const key of ['items','results','entries','jobs','hits']) if (Array.isArray(object[key])) return records(object[key]); }
  return [];
}
function display(value: JsonValue | undefined): string {
  if (value == null) return '—';
  if (typeof value === 'object') return Array.isArray(value) ? value.map(display).join(', ') : Object.entries(value).map(([k,v]) => `${k}: ${display(v)}`).join('; ');
  return String(value);
}
function keyOf(row: Row): string { return display(row.id ?? row.name ?? row.messageId ?? row.digest); }
export function LearningPanel({pluginId,onBack}:{pluginId:string;onBack:()=>void}) {
  const [sessions,setSessions]=useState<SessionInfo[]>([]), [sessionId,setSessionId]=useState(''), [userScope,setUserScope]=useState(false);
  const [output,setOutput]=useState('');
  const [query,setQuery]=useState(''), [rows,setRows]=useState<Row[]>([]), [selected,setSelected]=useState<Row|null>(null), [busy,setBusy]=useState(false), [error,setError]=useState('');
  const [models,setModels]=useState<{value:string;label:string}[]>([]), [mode,setMode]=useState('parallel');
  const [title,setTitle]=useState(''), [body,setBody]=useState(''), [model,setModel]=useState(''), [budget,setBudget]=useState('8'), [k,setK]=useState('1');
  useEffect(()=>{ let active=true; invoke('session_list').then(v=>{if(active){setSessions(v);setSessionId(old=>old || v[0]?.id || '');}}).catch(e=>{if(active)setError(String(e));}); return ()=>{active=false;}; },[]);
  useEffect(()=>{let active=true;invoke('model_providers').then(p=>{if(active)setModels(p.flatMap(x=>x.models.map(m=>({value:`api:${x.id}/${m}`,label:`${x.name} / ${m}`}))));}).catch(e=>{if(active)setError(String(e));});return()=>{active=false;};},[]);
  const call=useCallback(async(method:string,payload:Row={})=>invoke('plugins.call',{id:pluginId,method,params:{sessionId,...(userScope?{scope:{kind:'user'}}:{}),payload}}),[pluginId,sessionId,userScope]);
  const refresh=useCallback(async()=>{if(!sessionId)return;setBusy(true);setError('');try {setRows(records(await call(listMethods[pluginId] ?? 'list',{query,limit:50})));setSelected(null);}catch(e){setRows([]);setError(String(e));}finally{setBusy(false);}},[call,pluginId,query,sessionId]);
  useEffect(()=>{void refresh();},[sessionId,userScope,pluginId]); // Search is submitted explicitly.
  async function open(row:Row){setBusy(true);setError('');try {const id=keyOf(row);const payload:Row = pluginId==='session-query' ? {source:row.source ?? row.sourceRef ?? row} : pluginId==='skills' ? {name:id} : {id};const result=await call(readMethods[pluginId] ?? 'get',payload);if(!result || typeof result!=='object' || Array.isArray(result)) throw new Error('Сервис вернул неожиданный ответ');setSelected(result as Row);}catch(e){setError(String(e));}finally{setBusy(false);}}
  async function inspect(method:string,payload:Row){setBusy(true);setError('');try{setOutput(display(await call(method,payload)));}catch(e){setError(String(e));}finally{setBusy(false);}}
  async function action(method:string,payload:Row){setBusy(true);setError('');try{await call(method,payload);const list=await call(listMethods[pluginId] ?? 'list',{query,limit:50});setRows(records(list));if(typeof payload.id==='string' && ['experiments','improvement','skill-curation'].includes(pluginId)){const item=await call('get',{id:payload.id});if(item && typeof item==='object' && !Array.isArray(item))setSelected(item as Row);}else setSelected(null);}catch(e){setError(String(e));}finally{setBusy(false);}}
  const id=selected ? keyOf(selected) : '';
  const selectedState=display(selected?.state ?? selected?.status);
  useEffect(()=>{if(!id || !['running','evaluating','preparing'].includes(selectedState))return;let live=true;let timer:ReturnType<typeof setTimeout>;async function poll(){try{const item=await call('get',{id});if(live && item && typeof item==='object' && !Array.isArray(item))setSelected(item as Row);}catch(e){if(live)setError(String(e));}if(live)timer=setTimeout(poll,1500);}timer=setTimeout(poll,1500);return()=>{live=false;clearTimeout(timer);};},[call,id,selectedState]);
  return <ScrollArea asChild><section className="plugins-screen learning-screen" aria-labelledby="learning-title">
    <div className="plugins-header"><div><Button variant="outline" onClick={onBack}>Назад</Button><h1 id="learning-title">{labels[pluginId] ?? pluginId}</h1></div><Button variant="outline" onClick={()=>void refresh()} disabled={busy || !sessionId}>Обновить</Button></div>
    <div className="learning-scope"><label>Чат <select value={sessionId} onChange={e=>setSessionId(e.target.value)}><option value="">Выберите чат</option>{sessions.map(s=><option key={s.id} value={s.id}>{s.title || s.id}</option>)}</select></label>{!sessions.length && <p>Нет доступных чатов.</p>}
    <label><input type="checkbox" checked={userScope} onChange={e=>setUserScope(e.target.checked)}/> Личная область</label></div>
    {error && <div role="alert" className="plugins-error">{error}</div>}{busy && <p role="status">Загрузка…</p>}
    <form onSubmit={e=>{e.preventDefault();void refresh();}}><label>Поиск <input value={query} onChange={e=>setQuery(e.target.value)} maxLength={512}/></label><Button disabled={busy || !sessionId}>Найти</Button></form>
    {!busy && sessionId && !rows.length && !error && <p className="plugins-empty">Записей пока нет.</p>}
    <div className="plugin-list">{rows.map((row,i)=><article className="plugin-row" key={`${keyOf(row)}-${i}`}><div className="plugin-details"><strong>{pluginId==='memory' ? (row.kind==='preference'?'Предпочтение':'Факт') : display(row.title ?? row.cue ?? row.name ?? row.text ?? row.id ?? row.messageId)}</strong><p>{display(row.summary ?? row.description ?? row.text ?? row.status)}</p></div><Button variant="outline" disabled={busy} onClick={()=>void open(row)}>Открыть</Button></article>)}</div>
    {output && <pre style={{whiteSpace:'pre-wrap'}}>{output}</pre>}
    {selected && <article className="plugin-row"><div className="plugin-details"><h2>Подробнее</h2>{Object.entries(selected).map(([key,value])=><p key={key}><strong>{key}:</strong> {display(value)}</p>)}
      {pluginId==='session-query' && <Button variant="outline" disabled={busy} onClick={()=>void inspect('trace',{source:selected.source ?? selected.sourceRef ?? selected})}>Показать источник</Button>}
      {pluginId==='memory' && <><Button disabled={busy} onClick={()=>void action('decide',{id,revision:selected.revision ?? 0,accept:true})}>Принять</Button> <Button variant="outline" disabled={busy} onClick={()=>void action('forget',{id})}>Забыть</Button> <Button variant="outline" disabled={busy} onClick={()=>void inspect('audit',{id})}>История изменений</Button></>}
      {pluginId==='experiments' && <><Button disabled={busy} onClick={()=>void action('start',{id})}>Запустить</Button> <Button variant="outline" disabled={busy} onClick={()=>void action('cancel',{id})}>Отменить</Button> <Button variant="outline" disabled={busy} onClick={()=>void action('resume',{id})}>Продолжить</Button></>}
      {pluginId==='skills' && <Button variant="outline" disabled={busy} onClick={()=>void action('rollback',{name:id,targetRevision:title,expectedRevision:display(selected.revision)})}>Откатить к указанной ревизии</Button>}
      {pluginId==='experience' && <Button variant="outline" disabled={busy} onClick={()=>void action('retire',{id})}>Отозвать</Button>}
      {pluginId==='experience' && <Button variant="outline" disabled={busy} onClick={()=>void inspect('export',{id})}>Экспорт</Button>}
      {pluginId==='experiments' && <Button variant="outline" disabled={busy} onClick={()=>void inspect('compare',{id})}>Сравнить результаты</Button>}
      {pluginId==='skill-curation' && <>{selectedState==='draft' && <Button variant="outline" disabled={busy} onClick={()=>void action('evaluate',{id})}>Проверить</Button>} {selectedState==='evaluating' && <Button variant="outline" disabled={busy} onClick={()=>void action('cancel',{id})}>Отменить проверку</Button>} {selectedState==='ready' && <Button disabled={busy} onClick={()=>void action('publish',{id})}>Опубликовать</Button>} {['draft','ready','evaluated'].includes(selectedState) && <Button variant="outline" disabled={busy} onClick={()=>void action('reject',{id})}>Отклонить</Button>} {selectedState==='published' && <Button variant="outline" disabled={busy} onClick={()=>void action('rollback',{id})}>Откатить</Button>} {selectedState==='pending_publish' && <Button variant="outline" disabled={busy} onClick={()=>void action('recover',{id})}>Восстановить публикацию</Button>}</>}
    </div></article>}
    {pluginId==='skill-curation' && <CurationForms sessionId={sessionId} userScope={userScope} busy={busy} call={call} refresh={refresh} report={setError}/>}
    {pluginId==='memory' && selected && <form onSubmit={e=>{e.preventDefault();void action('upsert',{writes:[{record:{...selected,text:body},expectedRevision:selected.revision}],requestId:crypto.randomUUID()});}}><h2>Изменить запись</h2><label>Новый текст <textarea required value={body} onChange={e=>setBody(e.target.value)}/></label><Button disabled={busy}>Сохранить с проверкой версии</Button></form>}
    {['memory','experiments','improvement','skills'].includes(pluginId) && <form onSubmit={e=>{e.preventDefault(); if(pluginId==='memory') void action('remember',{text:body,requestId:crypto.randomUUID()}); else if(pluginId==='skills') void action('publish',{name:title,body,expectedRevision:selected?.revision ?? null}); else void action(pluginId==='experiments'?'create':'start',{task:title,model,budget:Number(budget),k:Number(k),mode});}}><h2>{pluginId==='memory'?'Добавить запись':pluginId==='skills'?'Публикация навыка':'Новая задача'}</h2>{pluginId!=='memory' && <label>{pluginId==='skills'?'Название':'Задача'} <input required maxLength={512} value={title} onChange={e=>setTitle(e.target.value)}/></label>}{['memory','skills'].includes(pluginId)?<label>Содержание <textarea required maxLength={8192} value={body} onChange={e=>setBody(e.target.value)}/></label>:<><label>Модель <select required value={model} onChange={e=>setModel(e.target.value)}><option value="">Выберите модель</option>{models.map(m=><option key={m.value} value={m.value}>{m.label}</option>)}</select></label>{pluginId==='improvement' && <label>Режим <select value={mode} onChange={e=>setMode(e.target.value)}><option value="parallel">Параллельный</option><option value="sequential">Последовательный</option></select></label>}<label>Лимит вызовов модели <input required type="number" min="1" max="1000" value={budget} onChange={e=>setBudget(e.target.value)}/></label><label>Число попыток <input required type="number" min="1" max={pluginId==='experiments'?1:5} value={k} onChange={e=>setK(e.target.value)}/></label></>}<Button disabled={busy || !sessionId}>Сохранить</Button></form>}
  </section></ScrollArea>;
}
