import test from 'node:test';
import assert from 'node:assert/strict';
import { createHttpClient } from '../packages/client/src/http.ts';
import { createClient } from '../packages/client/src/index.ts';
import { createTauriClient } from '../packages/client/src/tauri.ts';

test('HTTP bootstrap authenticates and RPC preserves camelCase nullable DTO fields',async()=>{
 const calls=[];
 const fixture={id:'s',title:'Hello',updatedAt:12,model:null};
 const client=await createHttpClient({fetch:async(url,init)=>{calls.push([url,init]);return Response.json(url.endsWith('bootstrap')?{protocolVersion:1,csrfToken:'token',principal:{id:'p'},workspace:{id:'w'},capabilities:['sessions']}:{ok:true,value:fixture});}});
 assert.deepEqual(await client.call('session_get',{id:'s'}),fixture);
 assert.equal(calls[0][1].credentials,'same-origin');
 assert.equal(calls[1][1].headers['X-Minerva-CSRF'],'token');
 assert.deepEqual(JSON.parse(calls[1][1].body),{protocolVersion:1,method:'session_get',params:{id:'s'}});
});
test('unsupported bootstrap versions fail before any RPC',async()=>{
 await assert.rejects(createHttpClient({fetch:async()=>Response.json({protocolVersion:2})}),e=>e.code==='unsupported_protocol');
});
test('structured errors preserve code message requestId and never become reply text',async()=>{
 const client=await createHttpClient({fetch:async url=>Response.json(url.endsWith('bootstrap')?{protocolVersion:1,csrfToken:'x',principal:{id:'p'},workspace:{id:'w'},capabilities:[]}:{ok:false,error:{code:'forbidden',message:'Denied',requestId:'req-1'}})});
 await assert.rejects(client.call('session_list',{}),e=>e.code==='forbidden'&&e.message==='Denied'&&e.requestId==='req-1');
});
test('unknown methods cannot reach an injected transport',async()=>{
 let reached=false;const client=createClient(async()=>{reached=true;});
 await assert.rejects(client.call('unknown',{}),e=>e.code==='unknown_method');assert.equal(reached,false);
});

test('optional plugin switch preserves its boolean through HTTP and Tauri transports',async()=>{
 const roster=[{id:'plugin-manager',enabled:true,optionalEnabled:false,requestedEnabled:true}];
 const native=[];
 const desktop=createTauriClient(async(...args)=>{native.push(args);return roster;});
 assert.deepEqual(await desktop.call('set_optional_plugins_enabled',{enabled:false}),roster);
 assert.deepEqual(native,[['set_optional_plugins_enabled',{enabled:false}]]);
 let request;
 const http=await createHttpClient({fetch:async(url,init)=>{
  if(url.endsWith('bootstrap'))return Response.json({protocolVersion:1,csrfToken:'token',principal:{id:'p'},workspace:{id:'w'},capabilities:[]});
  request=JSON.parse(init.body);return Response.json({ok:true,value:roster});
 }});
 assert.deepEqual(await http.call('set_optional_plugins_enabled',{enabled:false}),roster);
 assert.equal(request.method,'set_optional_plugins_enabled');
 assert.deepEqual(request.params,{enabled:false});
});

test('removed Codex endpoints cannot reach the transport',async()=>{
 let calls=0;const client=createClient(async()=>{calls++;});
 for(const method of ['codex_models','codex_status','codex_quota','codex_login','codex_login_status']){
  await assert.rejects(client.call(method,{}),e=>e.code==='unknown_method');
 }
 assert.equal(calls,0);
});
test('malformed success envelope fails closed',async()=>{
 const client=await createHttpClient({fetch:async url=>Response.json(url.endsWith('bootstrap')?{protocolVersion:1,csrfToken:'x',principal:{id:'p'},workspace:{id:'w'},capabilities:[]}:{ok:true})});
 await assert.rejects(client.call('session_list',{}),e=>e.code==='invalid_response');
});

test('transport-independent client preserves tool enum tags and JSON null by callId',async()=>{
 const reply={parts:[{kind:'tool-call',data:{callId:'one',name:'shell',arguments:{command:'echo hi'}}},{kind:'tool-result',data:{callId:'one',output:null,isError:false}}]};
 const client=createClient(async()=>reply);
 assert.deepEqual(await client.call('session_run',{id:'s',request:{model:'m',lastMessageId:'u',responseId:'a',createdAt:'2026-11-07T00:00:00Z',options:{}}}),reply);
});
test('bootstrap rejects an anonymous principal and never exposes a usable client',async()=>{
 await assert.rejects(createHttpClient({fetch:async()=>Response.json({protocolVersion:1,csrfToken:'x',principal:null,workspace:{id:'w'},capabilities:[]})}),e=>e.code==='invalid_response');
});
test('HTTP authentication and network failures are stable client errors',async()=>{
 await assert.rejects(createHttpClient({fetch:async()=>new Response('',{status:401,headers:{'X-Request-ID':'auth-1'}})}),e=>e.code==='http_401'&&e.requestId==='auth-1');
 await assert.rejects(createHttpClient({fetch:async()=>{throw Error('secret transport details');}}),e=>e.code==='network_error'&&!e.message.includes('secret'));
});
test('HTTP non-success status preserves structured RPC error details',async()=>{
 const client=await createHttpClient({fetch:async url=>url.endsWith('bootstrap')?Response.json({protocolVersion:1,csrfToken:'x',principal:{id:'p'},workspace:{id:'w'},capabilities:[]}):Response.json({ok:false,error:{code:'permission_denied',message:'No access',requestId:'denied-1'}},{status:403})});
 await assert.rejects(client.call('session_list',{}),e=>e.code==='permission_denied'&&e.requestId==='denied-1');
});
test('local launch token is removed before exchange and never enters any request URL',async()=>{
 const events=[];const location={href:'http://127.0.0.1:1420/chat?view=one#minerva-token=launch-secret'};
 const history={state:{view:1},replaceState:(state,unused,url)=>{events.push(['clear',url]);location.href=String(url);}};
 const client=await createHttpClient({location,history,fetch:async(url,init)=>{
  events.push(['request',url,init]);assert.ok(!location.href.includes('launch-secret'));assert.ok(!url.includes('launch-secret'));
  if(url.endsWith('exchange'))return new Response(null,{status:204});
  return Response.json({protocolVersion:1,csrfToken:'csrf',principal:{id:'p'},workspace:{id:'w'},capabilities:[]});
 }});
 assert.equal(client.bootstrap.principal.id,'p');assert.equal(events[0][0],'clear');
 assert.equal(events[0][1],'http://127.0.0.1:1420/chat?view=one');
 assert.equal(events[1][1],'/api/v1/auth/exchange');
 assert.deepEqual(JSON.parse(events[1][2].body),{token:'launch-secret'});
 assert.equal(events[1][2].credentials,'same-origin');
 assert.equal(events[2][1],'/api/v1/bootstrap');
});
test('failed token exchange leaves URL cleared and never starts bootstrap',async()=>{
 const location={href:'http://localhost/#minerva-token=bad-secret'};const calls=[];
 await assert.rejects(createHttpClient({location,history:{state:null,replaceState:(_,__,url)=>{location.href=String(url);}},fetch:async(url)=>{calls.push(url);assert.equal(location.href,'http://localhost/');return new Response('',{status:401});}}),e=>e.code==='http_401');
 assert.deepEqual(calls,['/api/v1/auth/exchange']);assert.equal(location.href,'http://localhost/');
});
test('client facade enforces endpoint arguments and inferred result types',async()=>{
 const {execFileSync}=await import('node:child_process');
 const tsc=new URL('../node_modules/typescript/bin/tsc',import.meta.url);
 execFileSync(process.execPath,[tsc.pathname.replace(/^\/(\w:)/,'$1'),'--noEmit','--strict','--skipLibCheck','--target','ES2022','--module','ESNext','--moduleResolution','Bundler','--allowImportingTsExtensions','packages/client/src/typecheck.fixture.ts'],{cwd:new URL('..',import.meta.url),stdio:'pipe'});
});
test('desktop maps supervised run methods to native command names without changing run scope',async()=>{
 const calls=[];
 const client=createTauriClient(async(command,args)=>{calls.push([command,args]);return {sessionId:'s',runId:'r'};});
 await client.call('runs.start',{sessionId:'s',requestId:'r',parentId:null,message:{id:'u',text:'hi'},model:'api:p/m',options:{}});
 assert.equal(calls[0][0],'run_start');
 assert.equal(calls[0][1].request.sessionId,'s');
 await client.call('runs.get',{sessionId:'s',runId:'r'});
 assert.deepEqual(calls[1],['run_get',{sessionId:'s',runId:'r'}]);
});

test('desktop routes LSP document requests through the shared runtime',async()=>{
 const calls=[];const client=createTauriClient(async(command,args)=>{calls.push([command,args]);return null;});
 const params={sessionId:'s',path:'src/main.rs',version:1,text:'unsaved'};
 await client.call('mh-lsp.open',params);
 assert.deepEqual(calls,[['harness_rpc',{method:'mh-lsp.open',params}]]);
});
