import test from 'node:test';
import assert from 'node:assert/strict';
import {SessionModel} from '../packages/client-models/src/session.ts';
import {createSessionStream,createDesktopSessionStream} from '../packages/client/src/streams.ts';
const session={id:'s',title:'S',updatedAt:1,model:null};
const message={id:'r',parentId:null,role:'assistant',createdAt:'2026-01-01',parts:[],extensions:{}};
const baseline=(cursor=0)=>({kind:'baseline',sessionId:'s',snapshot:{session,messages:[message],cursor,activeRun:null}});

test('desktop observation resumes snapshots and fences late IPC after leaving a session',async()=>{
 const pending=[],frames=[];
 const stream=createDesktopSessionStream(async(method,params)=>{
  assert.equal(method,'session.snapshot');assert.equal(params.sessionId,'s');
  return new Promise(resolve=>pending.push(resolve));
 });
 const handle=await stream.follow('s',frame=>frames.push(frame));
 pending.shift()(baseline().snapshot);await new Promise(resolve=>setImmediate(resolve));
 assert.equal(frames[0].kind,'baseline');
 handle.reconnect();handle.dispose();pending.shift()(baseline(1).snapshot);
 await new Promise(resolve=>setImmediate(resolve));assert.equal(frames.length,1);
 const returned=await stream.follow('s',frame=>frames.push(frame));
 pending.shift()(baseline(2).snapshot);await new Promise(resolve=>setImmediate(resolve));
 assert.equal(frames[1].snapshot.cursor,2);returned.dispose();
});

test('message events keep a conversation visible even after legacy session updates',async()=>{
 let receive;
 const model=new SessionModel('s',{follow:async(_,cb)=>{receive=cb;return {dispose(){},reconnect(){}}}});
 await model.follow();
 receive({...baseline(),snapshot:{...baseline().snapshot,session:{...session,hasMessages:false},messages:[]}});
 receive({kind:'event',sessionId:'s',seq:1,eventType:'message.upsert',data:{...message,role:'user',parts:[{kind:'file',data:{attachmentId:'file'}}]}});
 assert.equal(model.getSnapshot().session.hasMessages,true);
 receive({kind:'event',sessionId:'s',seq:2,eventType:'session.updated',data:session});
 assert.equal(model.getSnapshot().session.hasMessages,true);
 model.dispose();
});
test('neutral extension JSON scalars survive a baseline without a reconnect loop',async()=>{
 let receive,reconnects=0;
 const model=new SessionModel('s',{follow:async(_,cb)=>{receive=cb;return {dispose(){},reconnect(){reconnects++}}}});
 await model.follow();const value=baseline();value.snapshot.messages=[{...message,parts:[{kind:'extension',data:null}]}];receive(value);
 assert.equal(model.getSnapshot().status,'ready');assert.equal(model.getSnapshot().messages[0].parts[0].data,null);assert.equal(reconnects,0);model.dispose();
});
test('not-found stream error ends observation instead of leaving loading state',async()=>{
 let receive,disposed=0;
 const model=new SessionModel('s',{follow:async(_,cb)=>{receive=cb;return {dispose(){disposed++},reconnect(){}}}});
 await model.follow();receive({kind:'error',sessionId:'s',code:'resource/not-found'});
 assert.equal(model.getSnapshot().status,'error');assert.equal(model.getSnapshot().error,'resource/not-found');assert.equal(disposed,1);
});
test('folds authoritative baselines, duplicates, gaps and execution without side effects',async()=>{
 let receive,reconnects=0,disposed=0;
 const model=new SessionModel('s',{follow:async(_,cb)=>{receive=cb;return {reconnect(){reconnects++},dispose(){disposed++}}}});
 await model.follow();receive(baseline());const stable=model.getSnapshot();
 receive({kind:'event',sessionId:'s',seq:0,eventType:'message.upsert',data:message});assert.equal(model.getSnapshot(),stable);
 receive({kind:'event',sessionId:'s',seq:1,eventType:'run.updated',data:{sessionId:'s',runId:'r',status:'interrupted',parts:[],error:null}});
 assert.equal(model.getSnapshot().messages[0].extensions.execution.status,'interrupted');
 receive({kind:'event',sessionId:'s',seq:3,eventType:'session.updated',data:{...session,title:'gap'}});assert.equal(reconnects,1);assert.equal(model.getSnapshot().session.title,'S');
 receive(baseline(3));assert.equal(model.getSnapshot().messages[0].extensions.execution,undefined);
 model.dispose();receive(baseline(4));assert.equal(model.getSnapshot().session,null);assert.equal(disposed,1);
});
test('late follow completion cannot reattach after disposal and invalid baseline is rejected',async()=>{
 let complete,receive,disposed=0;const model=new SessionModel('s',{follow:(_,cb)=>{receive=cb;return new Promise(r=>complete=r)}});
 const pending=model.follow();receive({...baseline(),snapshot:{...baseline().snapshot,messages:[{}]}});assert.equal(model.getSnapshot().session,null);
 model.dispose();complete({dispose(){disposed++},reconnect(){}});await pending;assert.equal(disposed,1);
});
test('socket reconnects with bounded backoff, fences old frames, and stops on auth close',async()=>{
 const sockets=[],timers=[],delays=[],frames=[];
 const stream=createSessionStream({url:'https://host/api/v1/streams?workspaceId=w',webSocketFactory:url=>{const socket={url,send(data){this.sent=JSON.parse(data)},close(){},onopen:null,onclose:null,onmessage:null,onerror:null};sockets.push(socket);return socket},setTimeout:(fn,ms)=>{timers.push(fn);delays.push(ms);return timers.length},clearTimeout:()=>{}});
 const handle=await stream.follow('s',frame=>frames.push(frame));const old=sockets[0];old.onopen();assert.equal(old.sent.kind,'open');assert.match(old.url,/^wss:/);
 old.onclose({code:1006});timers.shift()();const current=sockets[1];current.onopen();old.onmessage({data:JSON.stringify({...baseline(),id:old.sent.id})});assert.equal(frames.length,0);
 current.onmessage({data:JSON.stringify({...baseline(),id:current.sent.id})});assert.equal(frames.length,1);
 current.onclose({code:4401});assert.equal(timers.length,0);assert.ok(delays[0]>=250);handle.dispose();
});
test('new follow generation fences both old callbacks and late handles',async()=>{
 const pending=[],callbacks=[];let disposed=0;
 const model=new SessionModel('s',{follow:(_,cb)=>{callbacks.push(cb);return new Promise(resolve=>pending.push(resolve));}});
 const first=model.follow(),second=model.follow();
 callbacks[1](baseline(7));callbacks[0](baseline(1));assert.equal(model.getSnapshot().cursor,7);
 pending[0]({dispose(){disposed++},reconnect(){}});await first;assert.equal(disposed,1);
 pending[1]({dispose(){disposed++},reconnect(){}});await second;
 const before=model.getSnapshot();callbacks[1]({kind:'event',sessionId:'s',seq:7,eventType:'session.updated',data:session});assert.equal(model.getSnapshot(),before);
 model.dispose();assert.equal(disposed,2);
});
test('retry exhaustion stops and disposal fences scheduled retry',async()=>{
 const sockets=[],timers=[],frames=[];let cancelled=0;
 const stream=createSessionStream({maxRetries:2,webSocketFactory:()=>{const s={send(){},close(){},onopen:null,onmessage:null,onclose:null,onerror:null};sockets.push(s);return s},setTimeout:fn=>{timers.push(fn);return timers.length},clearTimeout:()=>cancelled++});
 const handle=await stream.follow('s',f=>frames.push(f));
 sockets[0].onclose({code:1006});timers.shift()();sockets[1].onclose({code:1006});timers.shift()();sockets[2].onclose({code:1006});assert.equal(timers.length,0);assert.equal(frames[0].kind,'disconnected');handle.dispose();
 const next=await stream.follow('s',()=>{});sockets[3].onclose({code:1006});next.dispose();timers.shift()();assert.equal(sockets.length,4);assert.equal(cancelled,1);
});
