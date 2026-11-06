import test from 'node:test';
import assert from 'node:assert/strict';
import {SessionsModel} from '../packages/client-models/src/sessions.ts';
import {Preferences} from '../packages/client-models/src/preferences.ts';
const item=(id,title=id)=>({id,title,model:null,updatedAt:1});
test('delayed list cannot roll back a newer event; reconnect replaces baseline',async()=>{
 let resolve;const model=new SessionsModel(()=>new Promise(r=>resolve=r));
 const pending=model.refresh();model.update(item('new'));resolve([item('old')]);await pending;
 assert.deepEqual(model.getSnapshot(),[item('new')]);
 const reconnect=model.reconnect();resolve([item('authoritative')]);await reconnect;
 assert.deepEqual(model.getSnapshot(),[item('authoritative')]);
});
test('disposed models fence pending requests and remove subscriptions',async()=>{
 let resolve;let calls=0;const model=new SessionsModel(()=>new Promise(r=>resolve=r));
 model.subscribe(()=>calls++);const pending=model.refresh();model.dispose();resolve([item('secret')]);await pending;
 assert.deepEqual(model.getSnapshot(),[]);assert.equal(calls,1);model.update(item('next'));assert.equal(calls,1);
});
test('preferences isolate server/principal/workspace and logout clears only its namespace',()=>{
 const map=new Map();const storage={get length(){return map.size},key:i=>[...map.keys()][i],getItem:k=>map.get(k)??null,setItem:(k,v)=>map.set(k,v),removeItem:k=>map.delete(k)};
 const scope={server:'http://one',principal:'alice',workspace:'a'};
 const a=new Preferences(storage,scope), b=new Preferences(storage,{...scope,workspace:'b'}), c=new Preferences(storage,{...scope,principal:'bob'});
 a.set('model','secret');b.set('model','b');c.set('model','c');assert.equal(a.get('model',''),'secret');a.clear();assert.equal(a.get('model',''),'');assert.equal(b.get('model',''),'b');assert.equal(c.get('model',''),'c');
});

test('overlapping refreshes use the newest requested baseline',async()=>{
 const pending=[];const model=new SessionsModel(()=>new Promise(resolve=>pending.push(resolve)));
 const first=model.refresh(),second=model.refresh();pending[0]([item('stale')]);await first;pending[1]([item('latest')]);await second;
 assert.deepEqual(model.getSnapshot(),[item('latest')]);
});
