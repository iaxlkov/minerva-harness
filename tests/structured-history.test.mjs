import test from 'node:test';
import assert from 'node:assert/strict';
import { transcriptContent } from '../apps/web/src/transcript.ts';
import { sessionHistory } from '../apps/web/src/sessionHistory.ts';
import { chatAdapter } from '../apps/web/src/chatAdapter.ts';
test('neutral extension scalars stay read-only data and malformed tool identities are rejected',()=>{
 assert.deepEqual(transcriptContent([{kind:'extension',data:null}]),[{type:'data',name:'provider-item',data:null}]);
 assert.throws(()=>transcriptContent([{kind:'tool-call',data:null}]),/вызов инструмента/);
});

const parts=[
  {kind:'text',data:{text:'Проверю два источника.'}},
  {kind:'tool-call',data:{callId:'one',name:'shell',arguments:{command:'echo привет'}}},
  {kind:'tool-call',data:{callId:'two',name:'mcp.lookup',arguments:{id:42}}},
  {kind:'tool-result',data:{callId:'two',output:{message:'offline'},isError:true}},
  {kind:'tool-result',data:{callId:'one',output:null,isError:false}},
  {kind:'text',data:{text:'Готово.'}},
];
const saved={id:'a',parentId:'u',role:'assistant',createdAt:'2026-11-06T12:00:00Z',parts,extensions:{execution:{status:'completed'}}};
test('parallel results join by call ID, preserving JSON null and error flags',()=>{
  const content=transcriptContent(parts);
  assert.equal(content.length,4);
  assert.deepEqual(content[1].result,{output:null});assert.equal(content[1].isError,false);
  assert.deepEqual(content[2].result,{output:{message:'offline'}});assert.equal(content[2].isError,true);
});
test('live reply and reopened conversation use identical structured content',async()=>{
  const history=sessionHistory('s',()=>{},()=>{},async()=>[saved]);
  const loaded=await history.load();
  assert.deepEqual(loaded.messages[0].message.content,transcriptContent(parts));
  assert.equal(loaded.messages[0].message.status.type,'complete');
});
test('interrupted pending call is restored without an invented result or execute callback',async()=>{
  const history=sessionHistory('s',()=>{},()=>{},async()=>[{...saved,parts:parts.slice(0,3),extensions:{execution:{status:'interrupted',error:'app closed'}}}]);
  const loaded=await history.load();
  assert.equal(loaded.messages[0].message.status.type,'incomplete');
  assert.equal(loaded.messages[0].message.content[1].result,undefined);
  assert.equal(loaded.messages[0].message.content[1].execute,undefined);
});
test('failed model request recovers durable tool effects and next user attaches to them',async()=>{
  const calls=[];
  const failed={...saved,parts:parts.slice(0,5),extensions:{execution:{status:'failed',error:'offline'}}};
  const ipc=async(command,args)=>{calls.push([command,args]);if(command==='session_run')throw Error('offline');if(command==='session_load')return [failed];};
  const history=sessionHistory('s',()=>{},()=>{},ipc);
  const adapter=chatAdapter({invoke:ipc,sessionId:'s',model:'api:test/model',options:{},history,onRunning:()=>{},onSaved:()=>{},onError:()=>{}});
  const user={id:'u',role:'user',createdAt:new Date(),content:[{type:'text',text:'start'}]};
  const reply=await adapter.run({messages:[user],abortSignal:new AbortController().signal,unstable_assistantMessageId:'a',unstable_getMessage:()=>({id:'a',createdAt:new Date()})});
  assert.equal(reply.status.type,'incomplete');assert.deepEqual(reply.content,transcriptContent(failed.parts));
  await history.append({parentId:'a',message:{...user,id:'next'}});
  assert.equal(calls.at(-1)[1].item.parentId,'a');
});
