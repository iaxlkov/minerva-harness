import React,{useEffect,useState,useSyncExternalStore} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {Pin,Archive,ArchiveRestore,X} from 'lucide-react';
import {Store} from '../../../packages/client-models/src/store';
import type {SessionInfo} from '../../../packages/client/src/types';
import {Button} from '../../../apps/web/src/components/ui/button';
import {ContextMenu} from '../../../apps/web/src/components/ContextMenu';
import {workspaceScope} from '../../../apps/web/src/app/plugins';
import {models} from '../../../apps/web/src/app/models';

export const sessionActions=new Store<{session:SessionInfo;action:'rename'|'select'|'archive'}|null>(null);

export function SessionMenu({session,workspaceId,pinned,archived,disabled,selected,onSelect,onArchived}:{session:SessionInfo;workspaceId:string;pinned:boolean;archived:boolean;disabled?:boolean;selected:boolean;onSelect:(item:SessionInfo)=>void;onArchived:()=>void}){
 const pending=useSyncExternalStore(sessionActions.subscribe,sessionActions.getSnapshot,sessionActions.getSnapshot);
 const [renaming,setRenaming]=useState(false),[title,setTitle]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
 useEffect(()=>{
  if(pending?.session.id!==session.id||pending.action!=='rename')return;
  sessionActions.set(null);
  if(pending.action==='rename'){setTitle(pending.session.title);setError('');setRenaming(true);}

 },[pending,session.id]);
 async function state(pin:boolean,archive:boolean){
  setBusy(true);setError('');
  try{await models.workspaces.setSessionState(session.id,pin,archive);if(archive)onArchived();}
  catch(error){setError(String(error));}finally{setBusy(false);}
 }
 async function rename(event:React.FormEvent){
  event.preventDefault();setBusy(true);setError('');
  try{await models.sessions.rename(session.id,title);setRenaming(false);}catch(error){setError(String(error));}finally{setBusy(false);}
 }
 return <>
  <ContextMenu target={{kind:'session',value:{session,workspaceId,pinned,archived,disabled:!!disabled||busy}}} scope={workspaceScope} label={`Меню диалога — ${session.title}`} disabled={disabled||busy} className={`session-row ${selected?'selected':''}`} triggerClassName="session-menu-trigger" onDialogOpen={renaming}>
   <Button variant="ghost" disabled={disabled||busy} className={`nav-item ${selected?'selected':''}`} title={session.title} onClick={()=>onSelect(session)}><span>{session.title}</span></Button>
   <Button variant="ghost" size="icon-sm" className="session-quick-action" disabled={disabled||busy} aria-label={`${archived?'Восстановить':'Архивировать'} диалог — ${session.title}`} onClick={()=>void state(pinned,!archived)}>{archived?<ArchiveRestore size={14}/>:<Archive size={14}/>}</Button>
   {!archived&&<Button variant="ghost" size="icon-sm" className={`session-quick-action ${pinned?'pinned':''}`} disabled={disabled||busy} aria-label={`${pinned?'Открепить':'Закрепить'} диалог — ${session.title}`} onClick={()=>void state(!pinned,false)}><Pin size={14}/></Button>}
  </ContextMenu>
  {error&&!renaming&&<p className="workspace-error" role="alert">{error}</p>}
  <Dialog.Root open={renaming} onOpenChange={value=>{if(!busy)setRenaming(value);}}><Dialog.Portal><Dialog.Overlay className="settings-overlay"/><Dialog.Content className="oe-studio workspace-action-dialog">
   <div className="workspace-action-header"><Dialog.Title>Переименовать диалог</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" disabled={busy} aria-label="Закрыть"><X size={16}/></Button></Dialog.Close></div>
   <Dialog.Description>Название диалога в истории.</Dialog.Description>
   <form className="workspace-action-form" onSubmit={event=>void rename(event)}><label>Название диалога<input autoFocus required maxLength={128} value={title} disabled={busy} onChange={event=>setTitle(event.target.value)} onFocus={event=>event.target.select()}/></label>{error&&<p role="alert">{error}</p>}
    <div className="workspace-action-footer"><Button type="button" variant="outline" disabled={busy} onClick={()=>setRenaming(false)}>Отмена</Button><Button type="submit" variant="outline" disabled={busy||!title.trim()}>Сохранить</Button></div>
   </form>
  </Dialog.Content></Dialog.Portal></Dialog.Root>
 </>;
}
