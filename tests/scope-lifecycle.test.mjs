import test from 'node:test';
import assert from 'node:assert/strict';
import {clientScope} from '../apps/web/src/client.ts';
import {clearClientState,registerScopeCleanup,replaceClientScope} from '../apps/web/src/app/models.ts';
test('workspace switch tears down observers and reboots through authenticated bootstrap without mutating credentials',()=>{
 const previous=globalThis.location;let target,closed=0;
 globalThis.location={href:'https://minerva.example/chat',assign:url=>{target=url;}};
 const identity={...clientScope};
 try{
  registerScopeCleanup(()=>{closed++;});
  replaceClientScope({...identity,workspace:'next-workspace'});
  assert.equal(closed,1);
  assert.equal(new URL(target).searchParams.get('workspaceId'),'next-workspace');
  assert.deepEqual(clientScope,identity);
  assert.throws(()=>replaceClientScope({...identity,principal:'forged'}),/authenticated login/);
  clearClientState();assert.equal(closed,1);
 }finally{globalThis.location=previous;}
});
