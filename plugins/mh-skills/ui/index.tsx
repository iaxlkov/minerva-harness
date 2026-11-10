import React,{useEffect,useState,useSyncExternalStore} from 'react';
import * as Dialog from '@radix-ui/react-dialog';
import {BookOpen,X} from 'lucide-react';
import type {BrowserPlugin,Scope} from '../../../packages/plugin-sdk/src';
import type {WorkspaceInfo} from '../../../packages/client/src/types';
import type {SettingsPageProps} from '../../../apps/web/src/app/plugins';
import {invoke} from '../../../apps/web/src/client';
import {Store} from '../../../packages/client-models/src/store';
import {Button} from '../../../apps/web/src/components/ui/button';
import './skills.css';

type Skill={name:string;description:string;providerId:string;instructionPath?:string;modelInvocable:boolean;userInvocable:boolean};
type Catalog={items:Skill[];incomplete:boolean;diagnostics:string[]};
export function SkillList({workspaceId}:{workspaceId?:string}){
 const [catalog,setCatalog]=useState<Catalog|null>(null),[error,setError]=useState(''),[revision,refresh]=useState(0);
 useEffect(()=>{
  let active=true;setCatalog(null);setError('');
  void invoke('plugins.call',{id:'skills',method:'catalog',params:workspaceId?{workspaceId}:{}}).then(value=>{if(active)setCatalog(value as unknown as Catalog);},reason=>{if(active)setError(String(reason));});
  return ()=>{active=false;};
 },[workspaceId,revision]);
 return <div className="skills-catalog"><div className="skills-toolbar"><span>{catalog?`Навыков: ${catalog.items.length}`:'Каталог навыков'}</span><Button variant="outline" onClick={()=>refresh(value=>value+1)}>Обновить</Button></div>
 {error?<p role="alert" className="settings-error">Не удалось загрузить навыки: {error}</p>:!catalog?<p role="status">Загрузка навыков…</p>:<>
 {catalog.incomplete&&<div role="alert" className="settings-error">Часть навыков недоступна.{catalog.diagnostics.map((text,index)=><p key={index}>{text}</p>)}</div>}
 {!catalog.items.length&&<p>{workspaceId?'В этом workspace нет локальных навыков.':'Глобальные навыки пока не подключены.'}</p>}
 {catalog.items.map(skill=><article className="skills-card" key={`${skill.providerId}:${skill.name}`}><strong>{skill.name}</strong>{skill.description&&<p>{skill.description}</p>}<small>{skill.instructionPath??skill.providerId}</small><span className="skills-access">{skill.modelInvocable?'Доступен модели':'Автовызов отключён'} · {skill.userInvocable?'Доступен пользователю':'Вызов пользователем отключён'}</span></article>)}
 </>}</div>;
}
function SkillsSettings({placement,selectedPage,onSelectPage}:SettingsPageProps){
 if(placement==='navigation')return <Button variant="ghost" className={selectedPage==='skills'?'selected':''} aria-current={selectedPage==='skills'?'page':undefined} onClick={()=>onSelectPage('skills')}><BookOpen size={16}/> Навыки</Button>;
 return <><h2>Глобальные навыки</h2><p>Общие навыки, подключённые для всех рабочих пространств.</p><SkillList/></>;
}
export function skillsPlugin(scope:Scope):BrowserPlugin{
 const selection=new Store<WorkspaceInfo|null>(null);
 function SkillsDialog(){
  const workspace=useSyncExternalStore(selection.subscribe,selection.getSnapshot);
  return <Dialog.Root open={!!workspace} onOpenChange={open=>{if(!open)selection.set(null);}}><Dialog.Portal><Dialog.Overlay className="settings-overlay"/><Dialog.Content className="oe-studio skills-dialog"><div className="skills-dialog-header"><Dialog.Title>Локальные навыки — {workspace?.title}</Dialog.Title><Dialog.Close asChild><Button variant="ghost" size="icon" aria-label="Закрыть"><X size={16}/></Button></Dialog.Close></div><Dialog.Description>Навыки выбранного рабочего пространства.</Dialog.Description>{workspace&&<SkillList key={workspace.id} workspaceId={workspace.id}/>}</Dialog.Content></Dialog.Portal></Dialog.Root>;
 }
 return {id:'skills-ui',requires:['minerva-ui'],apply(ctx){const scoped=ctx.scope(scope);
  scoped.slots.register('settings.pages',{entryId:'skills',order:200,component:SkillsSettings});
  scoped.slots.register('shell.content.after',{entryId:'skills-dialog',component:SkillsDialog});
  scoped.slots.register('contextMenu.icon',{entryId:'workspace.skills',component:()=> <BookOpen size={14}/>});
  scoped.contextMenus.register({id:'workspace.skills',target:'workspace',order:200,resolve:({workspace,disabled})=>[{id:'skills',label:'Локальные навыки',icon:{key:'workspace.skills'},disabled,run:()=>selection.set(workspace)}]});
  ctx.effect(()=>selection.set(null));
 }};
}
