import type {ShellNavigationProps,ShellSectionProps} from '../../../../packages/plugin-sdk/src/slots.types';
import {workspacePlugin} from '../../../../plugins/mh-workspaces/ui';
import React, {createContext, useContext} from 'react';
import type {ReactNode} from 'react';
import {BrowserPluginHost, type Scope, type SlotName, type SlotSelection, type BrowserPluginContext} from '../../../../packages/plugin-sdk/src/index.ts';
import {SlotRenderer} from '../../../../packages/plugin-sdk/src/renderer.tsx';
import type {Provider,SessionInfo,PluginModel,WorkspaceInfo} from '../../../../packages/client/src/types.ts';
import type {ToolCallMessagePartProps} from '@assistant-ui/react';
import type {ExternalClientModules} from '../../../../packages/plugin-sdk/src/client-modules.ts';
import {ExternalPluginErrors} from './ExternalPluginErrors';
import {models} from './models';
export const workspaceScope:{kind:'workspace';workspaceId:string} = {kind:'workspace',workspaceId:'default'};
export const sessionScope=(sessionId:string):Scope=>({kind:'session',workspaceId:workspaceScope.workspaceId,sessionId});
const HostContext=createContext<BrowserPluginHost|null>(null);
export function useUiPlugins(){const host=useContext(HostContext);if(!host)throw Error('Browser plugin composition is missing');return host;}
export interface SettingsPageProps {placement:'navigation'|'body'; selectedPage:string; onSelectPage:(id:string)=>void}
declare module '../../../../packages/plugin-sdk/src/context-menus.ts' {
 interface ContextMenuTargetMap {
  workspace:{workspace:WorkspaceInfo;disabled:boolean};
  session:{session:SessionInfo;workspaceId:string;pinned:boolean;archived:boolean;disabled:boolean};
 }
}
/** Public UI contracts contain display data and callbacks, never a plugin context. */
export type ShellSlotProps=ShellNavigationProps;
export interface AppSlotProps {
 'shell.sections':ShellSectionProps;
 'shell.rail.start':ShellSlotProps;
 'shell.rail.end':ShellSlotProps;
 'shell.sidebar.header':ShellSlotProps;
 'shell.sidebar.footer':ShellSlotProps;
 'shell.sidebar.content':ShellSlotProps;
 'shell.header.actions':ShellSlotProps;
 'shell.content.before':ShellSlotProps;
 'shell.content.after':ShellSlotProps;
 'root.layout':{children:ReactNode};
 'navigation.items':{session:SessionInfo;sessions:SessionInfo[];disabled?:boolean;onNew:(workspaceId?:string)=>void;onSession:(session:SessionInfo)=>void};
 'settings.pages':SettingsPageProps;
 'conversation.views':{session:SessionInfo;model:string;configured:boolean;providers:Provider[];pluginModels?:PluginModel[];onSelect:(model:string)=>void};
 'composer.actions':{model:string;onSelect:(model:string)=>void};
 'composer.menu':{sessionId:string;disabled:boolean;onClose:()=>void};
 'tool.renderers':ToolCallMessagePartProps;
 'message.renderers':{children:ReactNode};
 'workspace.panels':{session:SessionInfo};
 'workspace.menu':{workspace:WorkspaceInfo;disabled:boolean;onClose:()=>void;onAction:(action:'rename'|'remove')=>void};
 'contextMenu.icon':{icon:{key:string;src?:string}};
}
export function registerUiSlot<K extends SlotName>(ctx:BrowserPluginContext,slot:K,entry:{entryId:string;order?:number;component:(props:AppSlotProps[K])=>ReactNode}){
 return ctx.slots.register(slot,entry);
}
export async function createBrowserComposition(selection:SlotSelection={}){
 const host=new BrowserPluginHost(selection);
 await host.activate([{id:'minerva-ui',apply(ctx){ctx.slots.register('root.layout',{entryId:'default',component:({children}:{children:ReactNode})=><>{children}</>});}}]);
 await host.activate([workspacePlugin(workspaceScope)]);
 return host;
}
export function UiSlot<K extends SlotName>({slot,scope,entryId,componentProps,fallback}: {slot:K;scope:Scope;entryId?:string;componentProps?:AppSlotProps[K];fallback?:ReactNode}){
 const host=useUiPlugins();
 return <SlotRenderer registry={host.slots} slot={slot} scope={scope} entryId={entryId} componentProps={componentProps as unknown as Readonly<Record<string,unknown>>} fallback={fallback} onOpenSettings={()=>document.querySelector<HTMLButtonElement>('.settings-open')?.click()} onDisable={id=>models.plugins.getSnapshot().some(plugin=>plugin.id===id&&plugin.origin==='installed')?models.plugins.toggle(id,false):host.disable(id)}/>;
}
export function PluginRoot({host,children,externalPlugins}:{host:BrowserPluginHost;children:ReactNode;externalPlugins?:ExternalClientModules}){
 return <HostContext.Provider value={host}>{externalPlugins&&<ExternalPluginErrors loader={externalPlugins}/>}<UiSlot slot="root.layout" scope={{kind:'root'}} componentProps={{children}} fallback={children}/></HostContext.Provider>;
}
