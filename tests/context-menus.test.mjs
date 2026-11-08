import test from 'node:test';
import assert from 'node:assert/strict';
import {BrowserPluginHost} from '../packages/plugin-sdk/src/context.ts';
const scope={kind:'workspace',workspaceId:'test'};
const target={kind:'workspace',value:{id:'clicked'}};
const action=(id,run=()=>{})=>({id,label:id,run});

test('context menu contributions are scoped, ordered and withdrawn with guarded callbacks',async()=>{
 const host=new BrowserPluginHost();let picked;
 await host.activate([{id:'first',apply(ctx){ctx.scope(scope).contextMenus.register({id:'z',target:'workspace',order:200,resolve:()=>[action('late')]});}},
  {id:'second',apply(ctx){ctx.scope(scope).contextMenus.register({id:'a',target:'workspace',order:100,resolve:value=>[action('early',()=>{picked=value.id;})]});}}]);
 const resolved=await host.contextMenus.resolve(scope,target,new AbortController().signal);
 assert.deepEqual(resolved.actions.map(a=>a.id),['early','late']);
 target.value.id='different';await resolved.actions[0].run();assert.equal(picked,'clicked');target.value.id='clicked';
 assert.equal((await host.contextMenus.resolve({...scope,workspaceId:'other'},target,new AbortController().signal)).actions.length,0);
 await host.disable('second');await assert.rejects(()=>resolved.actions[0].run(),/unavailable/);
 assert.equal((await host.contextMenus.resolve(scope,target,new AbortController().signal)).actions.length,1);
 await host.dispose();
});

test('unloading and dismissal abort preparation and discard late results',async()=>{
 const host=new BrowserPluginHost();let finish,signal;
 await host.activate([{id:'async',apply(ctx){ctx.scope(scope).contextMenus.register({id:'async',target:'workspace',resolve:(_,s)=>{signal=s;return new Promise(resolve=>finish=resolve);}});}}]);
 const resolving=host.contextMenus.resolve(scope,target,new AbortController().signal);
 await new Promise(resolve=>setImmediate(resolve));await host.disable('async');
 assert.equal(signal.aborted,true);finish([action('late')]);assert.deepEqual((await resolving).actions,[]);
 await host.activate([{id:'async',apply(ctx){ctx.scope(scope).contextMenus.register({id:'async',target:'workspace',resolve:(_,s)=>{signal=s;return new Promise(()=>{});}});}}]);
 const abort=new AbortController();const dismissed=host.contextMenus.resolve(scope,target,abort.signal);
 await new Promise(resolve=>setImmediate(resolve));abort.abort();
 assert.deepEqual(await dismissed,{actions:[],errors:[]});assert.equal(signal.aborted,true);await host.dispose();
});

test('provider failures are isolated and duplicate registrations roll back',async()=>{
 const host=new BrowserPluginHost();
 await host.activate([{id:'good',apply(ctx){ctx.scope(scope).contextMenus.register({id:'good',target:'workspace',resolve:()=>[action('good')]});}},
  {id:'bad',apply(ctx){ctx.scope(scope).contextMenus.register({id:'bad',target:'workspace',resolve:()=>{throw Error('failed');}});}}]);
 await assert.rejects(host.activate([{id:'duplicate',apply(ctx){ctx.scope(scope).contextMenus.register({id:'good',target:'workspace',resolve:()=>[]});}}]),/Duplicate/);
 const result=await host.contextMenus.resolve(scope,target,new AbortController().signal);
 assert.equal(result.actions[0].id,'good');assert.match(result.errors[0],/bad.*failed/);await host.dispose();
});

test('slow providers time out without hiding healthy actions',async()=>{
 const host=new BrowserPluginHost();let signal;
 await host.activate([{id:'test',apply(ctx){
  ctx.scope(scope).contextMenus.register({id:'slow',target:'workspace',resolve:(_,s)=>{signal=s;return new Promise(()=>{});}});
  ctx.scope(scope).contextMenus.register({id:'fast',target:'workspace',resolve:()=>[action('fast')]});
 }}]);
 const result=await host.contextMenus.resolve(scope,target,new AbortController().signal);
 assert.equal(signal.aborted,true);assert.equal(result.actions[0].id,'fast');assert.match(result.errors[0],/timed out/);await host.dispose();
});
