import test from 'node:test';
import assert from 'node:assert/strict';
import {pollSession} from '../apps/web/src/harness/poll.ts';
import {attachmentAdapter} from '../apps/web/src/harness/attachments.ts';

test('session polling never overlaps and ignores results after disposal',async()=>{
 let resolve,active=0,max=0,received=[];
 const stop=pollSession(async()=>{max=Math.max(max,++active);await new Promise(r=>resolve=r);active--;return 1;},v=>received.push(v),()=>{},1);
 await new Promise(r=>setTimeout(r,10));stop();resolve();await new Promise(r=>setTimeout(r,10));
 assert.equal(max,1);assert.deepEqual(received,[]);
});
test('attachment upload stores only durable references in composer content',async()=>{
 const ref={id:'image',name:'pixel.png',mimeType:'image/png',bytes:3};const calls=[];
 const adapter=attachmentAdapter('session',async(method,args)=>{calls.push([method,args]);return ref;});
 const pending=await adapter.add({file:new File(['png'],'pixel.png',{type:'image/png'})});
 const complete=await adapter.send(pending);
 assert.equal(calls[0][0],'attachments.put');assert.equal(calls[0][1].sessionId,'session');
 assert.deepEqual(complete.content[0].providerMetadata.minerva.attachment,ref);
 assert.equal(JSON.stringify(complete.content).includes('base64'),false);
 assert.equal((await adapter.add({file:new File(['svg'],'drawing.svg',{type:'image/svg+xml'})})).type,'file');
 await assert.rejects(()=>adapter.add({file:new File([new Uint8Array(10*1024*1024+1)],'large.bin')}),/10/);
});
test('generic files use opaque references and preserve original upload bytes',async()=>{
 const bytes=new Uint8Array([0,255,128,42]);let uploaded;
 const adapter=attachmentAdapter('s',async(method,args)=>{uploaded=args;return {id:'file',name:args.name,mimeType:args.mimeType,bytes:4};});
 const file=await adapter.add({file:new File([bytes],'unknown.custom')});
 assert.equal(file.type,'file');assert.equal(file.contentType,'application/octet-stream');
 const sent=await adapter.send(file);
 assert.equal(uploaded.data,Buffer.from(bytes).toString('base64'));
 assert.equal(sent.content[0].type,'file');assert.equal(sent.content[0].data,'');
 assert.equal(sent.content[0].providerMetadata.minerva.attachment.id,'file');
});
