import test from 'node:test';
import assert from 'node:assert/strict';
import {chatAdapter} from '../apps/web/src/chatAdapter.ts';
import {sessionHistory} from '../apps/web/src/sessionHistory.ts';

const user={id:'u',role:'user',createdAt:new Date('2026-11-07T00:00:00Z'),content:[{type:'text',text:'hello'}]};
const args=controller=>({messages:[user],abortSignal:controller.signal,unstable_assistantMessageId:'a',unstable_getMessage:()=>({id:'a',createdAt:new Date()})});
test('server-owned chat atomically starts once and reads the durable result without saving a user twice',async()=>{
 const calls=[],errors=[];let reads=0;
 const ipc=async(method,params)=>{
  calls.push([method,params]);
  if(method==='runs.start')return {sessionId:'s',runId:'a'};
  if(method==='runs.get')return {sessionId:'s',runId:'a',status:++reads===1?'running':'completed',parts:[{kind:'text',data:{text:reads===1?'par':'answer'}}],error:null};
  if(method==='session_load')return [];
  throw Error(`unexpected ${method}`);
 };
 const history=sessionHistory('s',()=>{},e=>errors.push(e),ipc,{serverOwned:true});
 await history.append({parentId:null,message:user});
 const adapter=chatAdapter({invoke:ipc,sessionId:'s',model:'api:p/m',options:{},history,onRunning:()=>{},onSaved:()=>{},onError:e=>errors.push(e),serverOwned:true});
 const replies=[];
 for await (const reply of adapter.run(args(new AbortController())))replies.push(reply);
 assert.deepEqual(calls.filter(([method])=>method==='runs.start').map(([,params])=>params),[{sessionId:'s',requestId:'a',parentId:null,message:{id:'u',text:'hello'},model:'api:p/m',options:{}}]);
 assert.equal(calls.some(([method])=>method==='session_save'),false);
 assert.equal(replies.at(-1).content[0].text,'answer');
 assert.deepEqual(errors,[]);
});

test('explicit abort cancels the matching server run and preserves its partial transcript',async()=>{
 const calls=[];let cancelled=false;
 const ipc=async(method,params)=>{
  calls.push([method,params]);
  if(method==='runs.start')return {sessionId:'s',runId:'a'};
  if(method==='runs.cancel'){cancelled=true;return true;}
  if(method==='runs.get')return {sessionId:'s',runId:'a',status:cancelled?'cancelled':'running',parts:[{kind:'text',data:{text:'partial'}}],error:cancelled?'cancelled':null};
  if(method==='session_load')return [];
  throw Error(`unexpected ${method}`);
 };
 const controller=new AbortController();
 const history=sessionHistory('s',()=>{},()=>{},ipc,{serverOwned:true});
 const adapter=chatAdapter({invoke:ipc,sessionId:'s',model:'api:p/m',options:{},history,onRunning:()=>{},onSaved:()=>{},onError:()=>{},serverOwned:true});
 const stream=adapter.run(args(controller));
 await stream.next();controller.abort();
 let last;
 for await (const reply of stream)last=reply;
 assert.deepEqual(calls.find(([method])=>method==='runs.cancel')[1],{sessionId:'s',runId:'a'});
 assert.equal(last.content[0].text,'partial');
 assert.equal(last.status.type,'incomplete');
});
