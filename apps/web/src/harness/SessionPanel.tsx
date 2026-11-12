import {useEffect,useState} from 'react';
import type {WorkspaceSettings,ToolDefinition} from '../../../../packages/client/src/types';
import {invoke} from '../client';
import {Button} from '../components/ui/button';
import {Input} from '../components/ui/input';
import {Switch} from '../components/ui/switch';
import './harness.css';
export function SessionPanel({sessionId}:{sessionId:string}){
 const [settings,setSettings]=useState<WorkspaceSettings|null>(null),[tools,setTools]=useState<ToolDefinition[]>([]),[busy,setBusy]=useState(false),[error,setError]=useState(''),[saved,setSaved]=useState(false);
 useEffect(()=>{let disposed=false;setSettings(null);void Promise.all([invoke('workspace.get',{sessionId}),invoke('tools.list',{sessionId})]).then(([value,tools])=>{if(!disposed){setSettings(value);setTools(tools);}}).catch(()=>{if(!disposed)setError('Не удалось загрузить настройки сессии.');});return ()=>{disposed=true;};},[sessionId]);
 async function save(event:React.FormEvent){event.preventDefault();if(!settings)return;setBusy(true);setError('');setSaved(false);try{const current=await invoke('workspace.get',{sessionId});setSettings(await invoke('workspace.save',{sessionId,settings:{...current,contextWindow:settings.contextWindow,autoCompact:settings.autoCompact}}));setTools(await invoke('tools.list',{sessionId}));setSaved(true);}catch{setError('Не удалось сохранить настройки сессии.');}finally{setBusy(false);}}
 const edit=(next:WorkspaceSettings)=>{setSaved(false);setSettings(next);};
 return <><h2>Сессия</h2><p>Контекст текущей сессии. Папка определяется её рабочим пространством.</p>{error&&<p role="alert">{error}</p>}
  {settings?<form className="provider-form" onSubmit={save}>
   <label>Окно контекста (токены)<Input required type="number" min={1024} max={2097152} disabled={busy} value={settings.contextWindow} onChange={event=>edit({...settings,contextWindow:Number(event.target.value)})}/><span className="ds-field-hint">От 1 024 до 2 097 152. Должно соответствовать выбранной модели.</span></label>
   <label className="ds-switch-row"><span>Автоматически сжимать контекст<small>Старые сообщения сворачиваются в сводку, когда окно почти заполнено.</small></span><Switch aria-label="Автоматически сжимать контекст" disabled={busy} checked={settings.autoCompact} onCheckedChange={autoCompact=>edit({...settings,autoCompact})}/></label>
   <div className="ds-form-actions">{saved&&<p role="status">Настройки сохранены.</p>}<Button disabled={busy} type="submit">Сохранить настройки сессии</Button></div>
  </form>:!error&&<p role="status">Загрузка сессии…</p>}
  <h3 className="ds-section-title">Доступные инструменты · {tools.length}</h3>
  {tools.length?<div className="tool-list">{tools.map(tool=><div key={tool.name}><strong>{tool.name}{tool.sideEffect?' · изменяет состояние':''}</strong><span>{tool.description}</span></div>)}</div>:<p className="ds-field-hint">Инструменты появятся после загрузки сессии.</p>}
 </>;
}
