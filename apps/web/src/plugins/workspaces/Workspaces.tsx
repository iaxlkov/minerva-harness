import React, {useEffect,useState} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {ChevronDown,ChevronRight,Folder,FolderPlus,Plus,X} from 'lucide-react';
import {Button} from '../../components/ui/button';
import {ScrollArea} from '../../components/ui/scroll-area';
import {SessionMenu,sessionActions} from './SessionMenu';
import {WorkspaceMenu} from './WorkspaceMenu';
import {models} from '../../app/models';
import type {AppSlotProps} from '../../app/plugins';

export function Workspaces(props:AppSlotProps['navigation.items']) {
 const {session,sessions,onNew,onSession,disabled}=props;
 const workspaces=React.useSyncExternalStore(models.workspaces.subscribe,models.workspaces.getSnapshot,models.workspaces.getSnapshot);
 const action=React.useSyncExternalStore(sessionActions.subscribe,sessionActions.getSnapshot,sessionActions.getSnapshot);
 const [showArchived,setShowArchived]=useState(()=>workspaces.some(item=>(item.archivedSessionIds??[]).includes(session.id)));
 useEffect(()=>{if(!action||action.action==='rename')return;sessionActions.set(null);if(action.action==='select')onSession(action.session);else if(action.session.id===session.id){const owner=workspaces.find(item=>item.sessionIds.includes(session.id));onNew(owner?.id);}},[action]);
 const [collapsed,setCollapsed]=useState<Set<string>>(new Set());
 const [busy,setBusy]=useState(false),[error,setError]=useState('');
 const [dialog,setDialog]=useState(false),[path,setPath]=useState('');
 useEffect(()=>{void models.workspaces.refresh().catch(error=>setError(String(error)));},[sessions]);
 async function create(folder:string){
  setBusy(true);setError('');
  try {
   const workspace=await models.workspaces.create(folder);
   setCollapsed(current=>{const next=new Set(current);next.delete(workspace.id);return next;});
   setDialog(false);
   const latest=sessions.find(item=>workspace.sessionIds.includes(item.id));
   if(latest)onSession(latest);else onNew(workspace.id);
  }catch(error){setError(String(error));}finally{setBusy(false);}
 }
 async function add(){
  setError('');
  if(!('__TAURI_INTERNALS__' in globalThis)){setDialog(true);return;}
  setBusy(true);
  try{
   const {open}=await import('@tauri-apps/plugin-dialog');
   const folder=await open({directory:true,multiple:false,title:'Выберите папку проекта'});
   if(folder)await create(folder);
  }catch(error){setError(String(error));}finally{setBusy(false);}
 }
 return <section className="workspaces" aria-label="Рабочие пространства">
  <div className="workspaces-heading"><span>Workspaces</span><Button variant="ghost" size="icon" title="Создать workspace" aria-label="Создать workspace" disabled={disabled||busy} onClick={()=>void add()}><FolderPlus size={16}/></Button></div>
  {error&&<p className="workspace-error" role="alert">{error}</p>}
  <ScrollArea className="workspace-list" aria-label="История диалогов">
   {workspaces.map(workspace=>{
    const title=workspace.title;
    const open=!collapsed.has(workspace.id);
    return <div className="workspace-group" key={workspace.id}>
     <WorkspaceMenu workspace={workspace} disabled={disabled||busy} onRemoved={()=>{if(workspace.sessionIds.includes(session.id))onNew('default');}}>
      <Button variant="ghost" className="workspace-toggle" aria-expanded={open} title={workspace.path??title} onClick={()=>setCollapsed(current=>{const next=new Set(current);if(open)next.add(workspace.id);else next.delete(workspace.id);return next;})}><Folder className="workspace-folder" size={16} aria-hidden="true"/>{open?<ChevronDown className="workspace-chevron" size={16} aria-hidden="true"/>:<ChevronRight className="workspace-chevron" size={16} aria-hidden="true"/>}<span>{title}</span></Button>
      <Button variant="ghost" size="icon-sm" title={`Новый диалог — ${title}`} aria-label={`Новый диалог — ${title}`} disabled={disabled||busy||workspace.missing} onClick={()=>{setCollapsed(current=>{const next=new Set(current);next.delete(workspace.id);return next;});onNew(workspace.id);}}><Plus size={14}/></Button>
     </WorkspaceMenu>
     {workspace.missing&&<p className="workspace-error">Папка недоступна</p>}
     {open&&<div className="workspace-sessions">{sessions.filter(item=>workspace.sessionIds.includes(item.id)&&((workspace.archivedSessionIds??[]).includes(item.id)===showArchived)&&(showArchived||item.id===session.id||item.hasMessages!==false||(workspace.pinnedSessionIds??[]).includes(item.id))).sort((a,b)=>Number((workspace.pinnedSessionIds??[]).includes(b.id))-Number((workspace.pinnedSessionIds??[]).includes(a.id))).map(item=><SessionMenu key={item.id} session={item} workspaceId={workspace.id} pinned={(workspace.pinnedSessionIds??[]).includes(item.id)} archived={(workspace.archivedSessionIds??[]).includes(item.id)} disabled={disabled||busy} selected={item.id===session.id} onSelect={onSession} onArchived={()=>{if(item.id===session.id)onNew(workspace.id);}}/>)}</div>}
    </div>;
   })}
  </ScrollArea>
  <Button variant="ghost" className="workspace-archive-toggle" aria-pressed={showArchived} onClick={()=>setShowArchived(value=>!value)}>{showArchived?'Вернуться к диалогам':'Архив диалогов'}</Button>
  <Dialog.Root open={dialog} onOpenChange={setDialog}><Dialog.Portal><Dialog.Overlay className="settings-overlay"/><Dialog.Content className="oe-studio workspace-dialog"><div className="settings-dialog-head"><Dialog.Title>Создать workspace</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label="Закрыть"><X size={16}/></Button></Dialog.Close></div><Dialog.Description>Укажите полный путь к папке на компьютере, где запущен Minerva. В приложении для ПК папка выбирается системным диалогом.</Dialog.Description><form className="provider-form" onSubmit={event=>{event.preventDefault();void create(path);}}><label>Папка проекта<input autoFocus required value={path} onChange={event=>setPath(event.target.value)} disabled={busy}/></label>{error&&<p role="alert">{error}</p>}<Button type="submit" disabled={busy||!path.trim()}>Создать</Button></form></Dialog.Content></Dialog.Portal></Dialog.Root>
 </section>;
}
