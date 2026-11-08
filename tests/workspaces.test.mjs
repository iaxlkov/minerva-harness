import test from 'node:test';
import assert from 'node:assert/strict';
import {WorkspacesModel} from '../packages/client-models/src/workspaces.ts';
import {SessionsModel} from '../packages/client-models/src/sessions.ts';
import {createTauriClient} from '../packages/client/src/tauri.ts';

test('new session reuses an empty current draft but never a conversation with messages',async()=>{
 const calls=[];let messages=[];
 const model=new SessionsModel(async(method,params)=>{
  calls.push([method,params]);
  if(method==='session_load')return messages;
  if(method==='session_get')return {id:params.id,title:'New',model:null,updatedAt:0};
 });
 const draft=await model.create('','default','draft');
 assert.equal(draft.id,'draft');assert.equal(calls.some(([m])=>m==='session_create'),false);
 messages=[{parts:[{kind:'file',data:{id:'attachment'}}]}];
 const next=await model.create('','default','draft');
 assert.notEqual(next.id,'draft');assert.equal(calls.filter(([m])=>m==='session_create').length,1);
});

test('workspace models reuse server identity and fence responses after scope disposal',async()=>{
 const project={id:'p',title:'Project',path:'C:/project',sessionIds:[],missing:false};
 let finish;
 const model=new WorkspacesModel(async(method)=>method==='workspaces.list'?[project]:await new Promise(resolve=>finish=resolve));
 await model.refresh();
 const pending=model.create('C:/project');model.dispose();finish(project);
 await assert.rejects(pending,/scope changed/);
 assert.deepEqual(model.getSnapshot(),[]);
});

test('selected workspace travels with session creation and shared desktop RPC',async()=>{
 const calls=[];
 const desktop=createTauriClient(async(command,params)=>{calls.push([command,params]);return command==='session_get'?{id:params.id,title:'New',updatedAt:0,model:null}:[];});
 const sessions=new SessionsModel((method,params={})=>desktop.call(method,params));
 await sessions.create('','project');
 assert.equal(calls[0][0],'session_create');
 assert.equal(calls[0][1].workspaceId,'project');
 await desktop.call('workspaces.create',{path:'C:/project'});
 assert.deepEqual(calls.at(-1),['harness_rpc',{method:'workspaces.create',params:{path:'C:/project'}}]);
 await desktop.call('session.snapshot',{sessionId:'active'});
 assert.deepEqual(calls.at(-1),['harness_rpc',{method:'session.snapshot',params:{sessionId:'active'}}]);
 for(const [method,params] of [['workspaces.rename',{id:'project',title:'Renamed'}],['workspaces.remove',{id:'project'}]]){
  await desktop.call(method,params);
  assert.deepEqual(calls.at(-1),['harness_rpc',{method,params}]);
 }
});
