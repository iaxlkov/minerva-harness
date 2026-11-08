import type {BrowserPlugin,Scope} from '../../../../../packages/plugin-sdk/src/index';
import {Workspaces} from './Workspaces';
import {Pencil,Trash2} from 'lucide-react';
import {workspaceActions} from './actions';
import {models} from '../../app/models';

export function workspacePlugin(scope:Scope):BrowserPlugin {
 return {id:'workspaces-ui',requires:['minerva-ui'],apply(ctx){
  ctx.scope(scope).slots.register('navigation.items',{entryId:'workspaces',order:0,component:Workspaces});
  const scoped=ctx.scope(scope);
  scoped.slots.register('contextMenu.icon',{entryId:'workspace.rename',component:()=> <Pencil size={14}/>});
  scoped.slots.register('contextMenu.icon',{entryId:'workspace.remove',component:()=> <Trash2 size={14}/>});
  scoped.contextMenus.register({id:'workspace.rename',target:'workspace',order:100,resolve:({workspace,disabled})=>[
   {id:'rename',label:'Переименовать',icon:{key:'workspace.rename'},disabled,run:()=>workspaceActions.set({workspace,action:'rename'})},
  ]});
  scoped.contextMenus.register({id:'workspace.remove',target:'workspace',order:300,resolve:({workspace,disabled})=>[
   {id:'remove',label:'Удалить workspace',icon:{key:'workspace.remove'},disabled:disabled||workspace.id==='default',danger:true,separatorBefore:true,run:()=>workspaceActions.set({workspace,action:'remove'})},
  ]});
  ctx.effect(()=>workspaceActions.set(null));
  ctx.effect(()=>models.workspaces.dispose());
 }};
}
