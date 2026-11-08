import type {BrowserPlugin,Scope} from '../../../../../packages/plugin-sdk/src/index';
import {Workspaces} from './Workspaces';
import {Pencil,Trash2,Pin,PinOff,GitFork,Archive,ArchiveRestore,Copy} from 'lucide-react';
import {workspaceActions} from './actions';
import {sessionActions} from './SessionMenu';
import {invoke} from '../../client';
import {models} from '../../app/models';

export function workspacePlugin(scope:Scope):BrowserPlugin {
 return {id:'workspaces-ui',requires:['minerva-ui'],apply(ctx){
  ctx.scope(scope).slots.register('navigation.items',{entryId:'workspaces',order:0,component:Workspaces});
  const scoped=ctx.scope(scope);
  scoped.slots.register('contextMenu.icon',{entryId:'workspace.rename',component:()=> <Pencil size={14}/>});
  scoped.slots.register('contextMenu.icon',{entryId:'workspace.remove',component:()=> <Trash2 size={14}/>});
  for(const [key,Icon] of Object.entries({pin:Pin,unpin:PinOff,fork:GitFork,archive:Archive,restore:ArchiveRestore,copy:Copy}))scoped.slots.register('contextMenu.icon',{entryId:`session.${key}`,component:()=> <Icon size={14}/>});
  scoped.contextMenus.register({id:'session.pin',target:'session',order:100,resolve:({session,pinned,archived,disabled})=>archived?[]:[
   {id:'pin',label:pinned?'Открепить диалог':'Закрепить диалог',icon:{key:pinned?'session.unpin':'session.pin'},disabled,run:()=>models.workspaces.setSessionState(session.id,!pinned,false)},
  ]});
  scoped.contextMenus.register({id:'session.rename',target:'session',order:200,resolve:({session,disabled})=>[
   {id:'rename',label:'Переименовать',icon:{key:'workspace.rename'},disabled,run:()=>sessionActions.set({session,action:'rename'})},
  ]});
  scoped.contextMenus.register({id:'session.fork',target:'session',order:300,resolve:async({session,disabled},signal)=>{
   const snapshot=await invoke('session.snapshot',{sessionId:session.id});signal.throwIfAborted();
   return [{id:'fork',label:'Создать ветку',icon:{key:'session.fork'},disabled:disabled||!!snapshot.activeRun,run:async()=>{
    const fork=await models.sessions.fork(session.id);await models.workspaces.refresh();sessionActions.set({session:fork,action:'select'});
   }}];
  }});
  scoped.contextMenus.register({id:'session.archive',target:'session',order:400,resolve:({session,pinned,archived,disabled})=>[
   {id:'archive',label:archived?'Восстановить диалог':'Архивировать диалог',icon:{key:archived?'session.restore':'session.archive'},disabled,run:async()=>{
    await models.workspaces.setSessionState(session.id,pinned,!archived);if(!archived)sessionActions.set({session,action:'archive'});
   }},
  ]});
  scoped.contextMenus.register({id:'session.copy',target:'session',order:500,resolve:({session})=>[
   {id:'copy',label:'Копировать ID диалога',icon:{key:'session.copy'},separatorBefore:true,run:()=>navigator.clipboard.writeText(session.id)},
  ]});
  scoped.contextMenus.register({id:'workspace.rename',target:'workspace',order:100,resolve:({workspace,disabled})=>[
   {id:'rename',label:'Переименовать',icon:{key:'workspace.rename'},disabled,run:()=>workspaceActions.set({workspace,action:'rename'})},
  ]});
  scoped.contextMenus.register({id:'workspace.remove',target:'workspace',order:300,resolve:({workspace,disabled})=>[
   {id:'remove',label:'Удалить workspace',icon:{key:'workspace.remove'},disabled:disabled||workspace.id==='default',danger:true,separatorBefore:true,run:()=>workspaceActions.set({workspace,action:'remove'})},
  ]});
  ctx.effect(()=>workspaceActions.set(null));
  ctx.effect(()=>sessionActions.set(null));
  ctx.effect(()=>models.workspaces.dispose());
 }};
}
