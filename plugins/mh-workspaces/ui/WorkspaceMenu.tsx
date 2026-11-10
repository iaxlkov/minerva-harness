import React,{useEffect,useState,useSyncExternalStore} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {X} from 'lucide-react';
import {Button} from '../../../apps/web/src/components/ui/button';
import {ContextMenu} from '../../../apps/web/src/components/ContextMenu';
import {workspaceActions} from './actions';
import {UiSlot,workspaceScope} from '../../../apps/web/src/app/plugins';
import {models} from '../../../apps/web/src/app/models';
import type {WorkspaceInfo} from '../../../packages/client/src/types';

export function WorkspaceMenu({workspace,disabled,children,onRemoved}:{workspace:WorkspaceInfo;disabled?:boolean;children:React.ReactNode;onRemoved:()=>void}){
 const [action,setAction]=useState<'rename'|'remove'|null>(null);
 const [title,setTitle]=useState(''),[busy,setBusy]=useState(false),[error,setError]=useState('');
 function request(action:'rename'|'remove'){setTitle(workspace.title);setError('');setAction(action);}
 const pending=useSyncExternalStore(workspaceActions.subscribe,workspaceActions.getSnapshot);
 useEffect(()=>{if(pending?.workspace.id===workspace.id){setTitle(pending.workspace.title);setError('');setAction(pending.action);workspaceActions.set(null);}},[pending,workspace.id]);
 async function submit(event:React.FormEvent){
  event.preventDefault();setBusy(true);setError('');
  try{
   if(action==='rename')await models.workspaces.rename(workspace.id,title);
   else if(action==='remove'){await models.workspaces.remove(workspace.id);onRemoved();}
   setAction(null);
  }catch(error){setError(String(error));}finally{setBusy(false);}
 }
 return <>
  <ContextMenu target={{kind:'workspace',value:{workspace,disabled:!!disabled||busy}}} scope={workspaceScope} label={`Меню workspace — ${workspace.title}`} disabled={disabled||busy} className="workspace-row" triggerClassName="workspace-menu-trigger" onDialogOpen={action!==null} legacy={onClose=><UiSlot slot="workspace.menu" scope={workspaceScope} componentProps={{workspace,disabled:!!disabled||busy,onClose,onAction:request}}/>}>
   {children}
  </ContextMenu>
  <Dialog.Root open={action!==null} onOpenChange={value=>{if(!value&&!busy)setAction(null);}}><Dialog.Portal><Dialog.Overlay className="settings-overlay"/><Dialog.Content className="oe-studio workspace-action-dialog">
   <div className="workspace-action-header"><Dialog.Title>{action==='rename'?'Переименовать workspace':'Удалить workspace'}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" disabled={busy} aria-label="Закрыть"><X size={16}/></Button></Dialog.Close></div>
   <Dialog.Description>{action==='rename'?'Изменится только название в списке. Папка останется прежней.':`«${workspace.title}» исчезнет из списка. Папка и история диалогов сохранятся. Повторное открытие папки вернёт workspace.`}</Dialog.Description>
   <form className="workspace-action-form" onSubmit={event=>void submit(event)}>
    {action==='rename'&&<label>Название workspace<input autoFocus required maxLength={128} disabled={busy} value={title} onChange={event=>setTitle(event.target.value)} onFocus={event=>event.target.select()}/></label>}
    {error&&<p role="alert">{error}</p>}
    <div className="workspace-action-footer"><Button type="button" variant="outline" disabled={busy} onClick={()=>setAction(null)}>Отмена</Button><Button type="submit" variant="outline" className={action==='remove'?'workspace-action-delete':undefined} disabled={busy||(action==='rename'&&!title.trim())}>{action==='rename'?'Сохранить':'Удалить workspace'}</Button></div>
   </form>
  </Dialog.Content></Dialog.Portal></Dialog.Root>
 </>;
}
