import test from 'node:test';
import assert from 'node:assert/strict';
import {PluginsModel} from '../packages/client-models/src/plugins.ts';

const plugin=(enabled,error=null)=>({id:'example',name:'Example',description:'',enabled,requestedEnabled:enabled,optionalEnabled:true,error,origin:'installed',canToggle:true,hasUi:true});
const deferred=()=>{let resolve,reject;const promise=new Promise((yes,no)=>{resolve=yes;reject=no;});return {promise,resolve,reject};};
const flush=()=>new Promise(resolve=>setImmediate(resolve));

test('master toggle fences stale polling and disposal, and releases polling after rejection',async()=>{
 const list=deferred(),toggle=deferred();let lists=0;
 const model=new PluginsModel((command,args)=>{
  if(command==='list_plugins'){lists++;return list.promise;}
  assert.equal(command,'set_optional_plugins_enabled');assert.deepEqual(args,{enabled:false});return toggle.promise;
 });
 const refreshing=model.refresh();const toggling=model.toggleOptional(false);
 list.resolve([plugin(true)]);await refreshing;await model.refresh();
 assert.equal(lists,1);assert.deepEqual(model.getSnapshot(),[]);
 const paused={...plugin(false),requestedEnabled:true,optionalEnabled:false};
 toggle.resolve([paused]);await toggling;assert.deepEqual(model.getSnapshot(),[paused]);
 await model.refresh();assert.equal(lists,2);
 const late=deferred();const disposed=new PluginsModel(()=>late.promise);
 const pending=disposed.toggleOptional(false);disposed.dispose();late.resolve([paused]);await pending;
 assert.deepEqual(disposed.getSnapshot(),[]);
 const recovering=new PluginsModel(async command=>{if(command==='set_optional_plugins_enabled')throw Error('offline');return [plugin(true)];});
 await assert.rejects(recovering.toggleOptional(false),/offline/);await recovering.refresh();
 assert.deepEqual(recovering.getSnapshot(),[plugin(true)]);
});

test('roster refresh cannot publish pre-toggle state while a toggle is pending',async()=>{
 const list=deferred(),toggle=deferred();let lists=0;
 const model=new PluginsModel(command=>command==='list_plugins'?(lists++,list.promise):toggle.promise);
 const refreshing=model.refresh();const toggling=model.toggle('example',false);
 list.resolve([plugin(true)]);await refreshing;
 assert.deepEqual(model.getSnapshot(),[]);
 await model.refresh();assert.equal(lists,1);
 toggle.resolve([plugin(false)]);await toggling;
 assert.deepEqual(model.getSnapshot(),[plugin(false)]);
});

test('old refresh cannot roll back completed toggle',async()=>{
 const list=deferred();
 const model=new PluginsModel(command=>command==='list_plugins'?list.promise:Promise.resolve([plugin(false)]));
 const refreshing=model.refresh();await model.toggle('example',false);
 list.resolve([plugin(true)]);await refreshing;
 assert.deepEqual(model.getSnapshot(),[plugin(false)]);
});

test('polls do not overlap; crash status arrives and disposal fences in-flight results',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});
 const calls=[];const model=new PluginsModel(()=>{const call=deferred();calls.push(call);return call.promise;});
 const stop=model.startPolling();
 t.mock.timers.tick(10000);assert.equal(calls.length,1);
 calls[0].resolve([plugin(true)]);await flush();
 t.mock.timers.tick(2000);assert.equal(calls.length,2);
 calls[1].resolve([plugin(false,'process exited')]);await flush();
 assert.equal(model.getSnapshot()[0].error,'process exited');
 t.mock.timers.tick(2000);assert.equal(calls.length,3);
 stop();calls[2].resolve([plugin(true)]);await flush();
 t.mock.timers.tick(10000);assert.equal(calls.length,3);
 assert.equal(model.getSnapshot()[0].enabled,false);
 model.dispose();assert.deepEqual(model.getSnapshot(),[]);
});

test('transport failure retains roster and exposes error until recovery',async t=>{
 t.mock.timers.enable({apis:['setTimeout']});
 let fail=true;const model=new PluginsModel(async()=>{if(fail)throw Error('offline');return [plugin(false)];});
 model.set([plugin(true)]);const stop=model.startPolling();await flush();
 assert.deepEqual(model.getSnapshot(),[plugin(true)]);
 assert.match(model.errors.getSnapshot(),/offline/);
 fail=false;t.mock.timers.tick(2000);await flush();
 assert.deepEqual(model.getSnapshot(),[plugin(false)]);assert.equal(model.errors.getSnapshot(),'');stop();
});

test('dispose fences pending toggle and failed refresh notifications',async()=>{
 const list=deferred(),toggle=deferred();
 const model=new PluginsModel(command=>command==='list_plugins'?list.promise:toggle.promise);
 const refreshing=model.refresh();const toggling=model.toggle('example',true);model.dispose();
 list.reject(Error('late failure'));await assert.rejects(refreshing,/late failure/);
 toggle.resolve([plugin(true)]);await toggling;
 assert.deepEqual(model.getSnapshot(),[]);assert.equal(model.errors.getSnapshot(),'');
});
