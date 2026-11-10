import type {BrowserPlugin,Scope} from '../../../packages/plugin-sdk/src';
import {SessionAccess} from './SessionAccess';
import {SessionControls} from './SessionControls';
import {ToolAccess} from './ToolAccess';

export function interactionsPlugin(scope:Scope):BrowserPlugin{return {id:'interactions-ui',requires:['minerva-ui'],apply(ctx){
 const slots=ctx.scope(scope).slots;
 slots.register('composer.access',{entryId:'access',component:({sessionId,disabled})=><SessionAccess key={sessionId} sessionId={sessionId} disabled={disabled}/>});
 slots.register('session.controls',{entryId:'controls',component:({sessionId,running})=><SessionControls inlineApprovals key={sessionId} sessionId={sessionId} running={running}/>});
 slots.register('tool.interactions',{entryId:'approval',component:({sessionId,callId})=><ToolAccess key={`${sessionId}:${callId}`} sessionId={sessionId} callId={callId}/>});
}};}
