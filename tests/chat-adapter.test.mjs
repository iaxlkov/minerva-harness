import test from 'node:test';
import assert from 'node:assert/strict';
import { chatAdapter } from '../src/chatAdapter.ts';
import { sessionHistory } from '../src/sessionHistory.ts';

const message = (id, role, text = '') => ({id, role, createdAt:new Date('2026-11-06T00:00:00Z'),content:[{type:'text',text}]});
function setup(ipc) {
  const errors=[], running=[], saved=[];
  const history=sessionHistory('s',()=>saved.push(true),e=>errors.push(e),ipc);
  const adapter=chatAdapter({invoke:ipc,sessionId:'s',model:'api:test/model',options:{},history,onRunning:v=>running.push(v),onSaved:()=>saved.push(true),onError:e=>errors.push(e)});
  const controller=new AbortController();
  const args={messages:[message('u','user','hello')],abortSignal:controller.signal,unstable_assistantMessageId:'a',unstable_getMessage:()=>message('a','assistant')};
  return {adapter,history,errors,running,saved,controller,args};
}
test('result uses backend-owned ID; history adapter never writes assistant',async()=>{
  const calls=[];
  const ctx=setup(async(cmd,args)=>{calls.push([cmd,args]);return {text:'answer',summary:null};});
  const result=await ctx.adapter.run(ctx.args);
  assert.equal(result.content[0].text,'answer');
  assert.equal(calls[1][1].request.responseId,'a');
  assert.deepEqual(ctx.running,[true,false]);
  await ctx.history.append({parentId:'u',message:message('a','assistant','answer')});
  assert.equal(calls.length,2);
});
test('failure throws instead of becoming model text; next user skips transient assistant',async()=>{
  const calls=[];
  const ctx=setup(async(cmd,args)=>{calls.push([cmd,args]);if(cmd==='session_run')throw new Error('offline');});
  await assert.rejects(ctx.adapter.run(ctx.args),/offline/);
  assert.equal(ctx.errors.length,1);
  await ctx.history.append({parentId:'a',message:message('u2','user','again')});
  assert.equal(calls.at(-1)[1].item.parentId,'u');
});
test('abort requests cancellation of matching backend run and releases UI',async()=>{
  let rejectRun, started;
  const ready=new Promise(resolve=>{started=resolve;});
  const calls=[];
  const ctx=setup(async(cmd,args)=>{
    calls.push([cmd,args]);
    if(cmd==='session_run')return new Promise((_,reject)=>{rejectRun=reject;started();});
    if(cmd==='session_cancel'){rejectRun(new Error('cancelled'));return true;}
  });
  const pending=ctx.adapter.run(ctx.args);
  await ready;ctx.controller.abort();
  await assert.rejects(pending,/cancelled/);
  assert.equal(calls.at(-1)[1].runId,'a');
  assert.deepEqual(ctx.running,[true,false]);
  assert.equal(ctx.errors.length,0);
});
test('rehydrates backend reply without UI-specific stored metadata',async()=>{
  const history=sessionHistory('s',()=>{},()=>{},async()=>[{id:'a',parentId:'u',role:'assistant',createdAt:'2026-11-06T00:00:00Z',parts:[{kind:'text',data:{text:'saved'}}],extensions:{}}]);
  const result=await history.load();
  assert.equal(result.messages[0].message.status.type,'complete');
  assert.equal(result.messages[0].message.content[0].text,'saved');
});
