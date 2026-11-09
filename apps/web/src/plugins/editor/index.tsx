import React, {Suspense, lazy} from 'react';
import {Code2} from 'lucide-react';
import type {BrowserPlugin,Scope} from '../../../../../packages/plugin-sdk/src/index';
import type {ShellSectionProps} from '../../../../../packages/plugin-sdk/src/slots.types';
import {Button} from '../../components/ui/button';
const EditorView=lazy(()=>import('./view'));
function Section(props:ShellSectionProps){
 if(props.placement==='rail')return <Button variant="ghost" size="icon-lg" className={`rail-button ${props.selected?'selected':''}`} aria-label="Разработка" aria-current={props.selected?'page':undefined} onClick={()=>props.onNavigate('mh-editor')}><Code2/></Button>;
 if(props.placement==='title')return <>Разработка</>;
 return <Suspense fallback={<div role="status">Загрузка редактора…</div>}><EditorView sessionId={props.sessionId} placement={props.placement}/></Suspense>;
}
export function editorPlugin(scope:Scope):BrowserPlugin {return {id:'mh-editor-ui',requires:['minerva-ui'],apply(ctx){ctx.scope(scope).slots.register('shell.sections',{entryId:'mh-editor',order:30,component:Section});}};}
