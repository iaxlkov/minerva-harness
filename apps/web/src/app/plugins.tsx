import React, {createContext, useContext} from 'react';
import type {ReactNode} from 'react';
import {BrowserPluginHost, type BrowserPlugin, type Scope, type SlotName, type SlotSelection, type BrowserPluginContext} from '../../../../packages/plugin-sdk/src/index.ts';
import {SlotRenderer} from '../../../../packages/plugin-sdk/src/renderer.tsx';
import type {Provider,CodexModel,SessionInfo} from '../../../../packages/client/src/types.ts';
import type {ToolCallMessagePartProps} from '@assistant-ui/react';
import {codexSettingsContribution} from '../pages/settings/CodexSettings';
export const workspaceScope:{kind:'workspace';workspaceId:string} = {kind:'workspace',workspaceId:'default'};
export const sessionScope=(sessionId:string):Scope=>({kind:'session',workspaceId:workspaceScope.workspaceId,sessionId});
const HostContext=createContext<BrowserPluginHost|null>(null);
export function useUiPlugins(){const host=useContext(HostContext);if(!host)throw Error('Browser plugin composition is missing');return host;}
export interface SettingsPageProps {placement:'navigation'|'body'; selectedPage:string; onSelectPage:(id:string)=>void}
/** Public UI contracts contain display data and callbacks, never a plugin context. */
export interface AppSlotProps {
 'root.layout':{children:ReactNode};
 'navigation.items':{session:SessionInfo;onNew:()=>void;onSession:(session:SessionInfo)=>void};
 'settings.pages':SettingsPageProps;
 'conversation.views':{session:SessionInfo;model:string;configured:boolean;providers:Provider[];codexModels:CodexModel[];onSelect:(model:string)=>void};
 'composer.actions':{model:string;onSelect:(model:string)=>void};
 'tool.renderers':ToolCallMessagePartProps;
 'message.renderers':{children:ReactNode};
 'workspace.panels':{session:SessionInfo};
}
export function registerUiSlot<K extends SlotName>(ctx:BrowserPluginContext,slot:K,entry:{entryId:string;order?:number;component:(props:AppSlotProps[K])=>ReactNode}){
 return ctx.slots.register(slot,entry);
}
const codexUi:BrowserPlugin={id:'codex-ui',apply(ctx){registerUiSlot(ctx.scope(workspaceScope),'settings.pages',{entryId:'codex',component:({placement,selectedPage,onSelectPage}:SettingsPageProps)=>{
 const item=codexSettingsContribution;
 return placement==='navigation'?<button className={selectedPage==='codex'?'selected':''} aria-current={selectedPage==='codex'?'page':undefined} onClick={()=>onSelectPage('codex')}><item.Icon size={16}/>{item.title}</button>:selectedPage==='codex'?<item.Component/>:null;
}});}};
export async function createBrowserComposition(selection:SlotSelection={}){
 const host=new BrowserPluginHost(selection);
 await host.activate([{id:'minerva-ui',apply(ctx){ctx.slots.register('root.layout',{entryId:'default',component:({children}:{children:ReactNode})=><>{children}</>});}}]);
 return host;
}
const reconciliations=new WeakMap<BrowserPluginHost,Promise<void>>();
export function reconcileBuiltinUi(host:BrowserPluginHost,activeIds:readonly string[]):Promise<void>{
 const next=(reconciliations.get(host)??Promise.resolve()).catch(()=>{}).then(async()=>{
  if(activeIds.includes('codex')){if(!host.activePluginIds.includes(codexUi.id))await host.activate([codexUi]);}
  else await host.disable(codexUi.id);
 });
 reconciliations.set(host,next);return next;
}
export function UiSlot<K extends SlotName>({slot,scope,entryId,componentProps,fallback}: {slot:K;scope:Scope;entryId?:string;componentProps?:AppSlotProps[K];fallback?:ReactNode}){
 const host=useUiPlugins();
 return <SlotRenderer registry={host.slots} slot={slot} scope={scope} entryId={entryId} componentProps={componentProps as unknown as Readonly<Record<string,unknown>>} fallback={fallback} onOpenSettings={()=>document.querySelector<HTMLButtonElement>('.settings-open')?.click()} onDisable={id=>{void host.disable(id);}}/>;
}
export function PluginRoot({host,children}:{host:BrowserPluginHost;children:ReactNode}){
 return <HostContext.Provider value={host}><UiSlot slot="root.layout" scope={{kind:'root'}} componentProps={{children}} fallback={children}/></HostContext.Provider>;
}
