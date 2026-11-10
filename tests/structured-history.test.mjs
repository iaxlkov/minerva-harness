import test from 'node:test';
import assert from 'node:assert/strict';
import { transcriptContent } from '../apps/web/src/transcript.ts';
import { sessionHistory, decodeSessionMessage } from '../apps/web/src/sessionHistory.ts';
import { chatAdapter } from '../apps/web/src/chatAdapter.ts';
import { toolState, toolCard } from '../apps/web/src/execution/toolModel.ts';

test('recovered interrupted runs cannot retain nonterminal tool states',()=>{
 for(const recorded of ['pending','running','requires-approval']){
  assert.equal(toolState({providerMetadata:{minerva:{status:recorded}},status:{type:'incomplete',reason:'error'}}).status,'interrupted');
  assert.equal(toolState({providerMetadata:{minerva:{status:recorded}},status:{type:'incomplete',reason:'cancelled'}}).status,'cancelled');
 }
 assert.equal(toolState({providerMetadata:{minerva:{status:'completed'}},result:{output:1},status:{type:'incomplete',reason:'error'}}).status,'completed');
});

test('tool state never infers interruption from a pending result',()=>{
  assert.equal(toolState({}).status,'pending');
  assert.equal(toolState({status:{type:'running'}}).status,'running');
  for(const status of ['pending','running','requires-approval','completed','failed','cancelled']){
    assert.equal(toolState({providerMetadata:{minerva:{status}}}).status,status);
  }
  assert.equal(toolState({result:{output:null}}).status,'completed');
  assert.equal(toolState({result:{output:'offline'},isError:true}).status,'failed');
});
test('tool cards use recorded structured results and unknown tools keep their raw output',()=>{
  assert.equal(toolCard('read_file',{path:'a.txt'},{content:'hello'}).kind,'read');
  assert.equal(toolCard('search',{pattern:'needle'},{matches:[]}).kind,'search');
  assert.equal(toolCard('apply_patch',{path:'a.txt'},{diff:'-old\n+new'}).kind,'diff');
  assert.equal(toolCard('shell',{command:'echo yes'},{stdout:'yes',stderr:'',exitCode:0}).kind,'shell');
  assert.equal(toolCard('unknown',{},null).kind,'generic');
});

test('native Rig JSON tool results populate specialized cards without changing their saved output',()=>{
 const output=[{type:'json',value:{path:'fixture.txt',content:'actual file'}}];
 const before=JSON.stringify(output),card=toolCard('read_file',{path:'fixture.txt'},output);
 assert.equal(card.result.content,'actual file');assert.equal(card.output,output);assert.equal(JSON.stringify(output),before);
 assert.equal(toolCard('patch_file',{},[{type:'json',value:{diff:'-old\n+new'}}]).result.diff,'-old\n+new');
 const multiple=[...output,{type:'text',text:'extra diagnostics'}];assert.equal(toolCard('unknown',{},multiple).output,multiple);
});
test('images project validated raster data and preserve durable references without accepting URLs',()=>{
  const ref={id:'image-1',name:'diagram.png',mimeType:'image/png',bytes:3};
  const [image]=transcriptContent([{kind:'image',data:{...ref,data:'YWJj',url:'https://untrusted.test/image'}}]);
  assert.equal(image.type,'image');assert.equal(image.image,'data:image/png;base64,YWJj');
  assert.deepEqual(image.providerMetadata.minerva.attachment,ref);
  assert.equal(transcriptContent([{kind:'image',data:ref}])[0].image,'');
  assert.throws(()=>transcriptContent([{kind:'image',data:{...ref,mimeType:'image/svg+xml',data:'bad'}}]),/изображение/);
});
test('completed image generation projects a session-owned image without trusting file paths',()=>{
 const attachment={id:'image-2',name:'generated.png',mimeType:'image/png',bytes:123};
 const parts=[{kind:'tool-call',data:{callId:'c',name:'generate_image',arguments:{prompt:'hello'}}},{kind:'tool-result',data:{callId:'c',output:{status:'completed',attachment,path:'C:/private.png'},isError:false}}];
 const result=transcriptContent(parts);
 assert.equal(result[0].type,'tool-call');assert.equal(result[1].type,'image');
 assert.equal(result[1].image,'');assert.deepEqual(result[1].providerMetadata.minerva.attachment,attachment);
 const wrapped=[parts[0],{...parts[1],data:{...parts[1].data,output:[{type:'json',value:parts[1].data.output}]}}];
 assert.equal(transcriptContent(wrapped)[1].type,'image');
 assert.equal(transcriptContent([{...parts[0]}, {...parts[1],data:{...parts[1].data,isError:true}}]).length,1);
 assert.equal(transcriptContent([{...parts[0]}, {...parts[1],data:{...parts[1].data,output:{status:'running',attachment}}}]).length,1);
});
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
test('live pending tools keep lifecycle and native metadata without an invented result',()=>{
  const [call]=transcriptContent([{kind:'tool-call',data:{callId:'pending',name:'shell',arguments:{},status:'pending',meta:{native:{signature:'opaque'}}}}]);
  assert.equal(call.providerMetadata.minerva.status,'pending');
  assert.deepEqual(call.providerMetadata.minerva.meta,{native:{signature:'opaque'}});
  assert.equal(call.result,undefined);
});
test('result pairing preserves explicit terminal status and separate call/result metadata',()=>{
  const content=transcriptContent([
    {kind:'tool-call',data:{callId:'a',name:'shell',arguments:{},status:'failed',meta:{native:'signature'}}},
    {kind:'tool-result',data:{callId:'a',output:null,isError:false,meta:{exitCode:1}}},
  ]);
  assert.deepEqual(content[0].providerMetadata.minerva,{status:'failed',meta:{native:'signature'},resultMeta:{exitCode:1}});
});
test('recovered running execution stays running, terminal errors and cancellation stay distinct',()=>{
  for(const [state,type,reason] of [['running','running',undefined],['completed','complete','stop'],['failed','incomplete','error'],['cancelled','incomplete','cancelled']]){
    const message=decodeSessionMessage({...saved,extensions:{execution:{status:state}}}).message;
    assert.equal(message.status.type,type);assert.equal(message.status.reason,reason);
  }
});
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
