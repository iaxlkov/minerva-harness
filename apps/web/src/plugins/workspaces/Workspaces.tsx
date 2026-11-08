import React, {useEffect,useState} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {ChevronDown,ChevronRight,Folder,FolderPlus,Plus,X} from 'lucide-react';
import {Button} from '../../components/ui/button';
import {ScrollArea} from '../../components/ui/scroll-area';
import {WorkspaceMenu} from './WorkspaceMenu';
import {models} from '../../app/models';
import type {AppSlotProps} from '../../app/plugins';

export function Workspaces(props:AppSlotProps['navigation.items']) {
 const {session,sessions,onNew,onSession,disabled}=props;
 const workspaces=React.useSyncExternalStore(models.workspaces.subscribe,models.workspaces.getSnapshot,models.workspaces.getSnapshot);
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
     {open&&<div className="workspace-sessions">{sessions.filter(item=>workspace.sessionIds.includes(item.id)&&(item.id===session.id||item.hasMessages!==false)).map(item=><Button variant="ghost" key={item.id} disabled={disabled||busy} className={`nav-item ${item.id===session.id?'selected':''}`} title={item.title} onClick={()=>onSession(item)}><span>{item.title}</span></Button>)}</div>}
    </div>;
   })}
  </ScrollArea>
  <Dialog.Root open={dialog} onOpenChange={setDialog}><Dialog.Portal><Dialog.Overlay className="settings-overlay"/><Dialog.Content className="oe-studio workspace-dialog"><div className="settings-dialog-head"><Dialog.Title>Создать workspace</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label="Закрыть"><X size={16}/></Button></Dialog.Close></div><Dialog.Description>Укажите полный путь к папке на компьютере, где запущен Minerva. В приложении для ПК папка выбирается системным диалогом.</Dialog.Description><form className="provider-form" onSubmit={event=>{event.preventDefault();void create(path);}}><label>Папка проекта<input autoFocus required value={path} onChange={event=>setPath(event.target.value)} disabled={busy}/></label>{error&&<p role="alert">{error}</p>}<Button type="submit" disabled={busy||!path.trim()}>Создать</Button></form></Dialog.Content></Dialog.Portal></Dialog.Root>
 </section>;
}
