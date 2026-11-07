import test from 'node:test';
import assert from 'node:assert/strict';
import * as React from 'react';
import { BrowserPluginHost } from '../packages/plugin-sdk/src/context.ts';
import { ExternalClientModules } from '../packages/plugin-sdk/src/client-modules.ts';
const enabled = [{ id: 'example', enabled: true, origin: 'installed', hasUi: true }];
const scope = { kind: 'workspace', workspaceId: 'w' };
const deferred = () => { let resolve; const promise = new Promise(r => resolve = r); return { promise, resolve }; };
const source = `exports.sdkVersion=2; exports.apply=ctx=>{ctx.slots.register('settings.pages',{entryId:'example',component:()=>null});};`;
function setup(load, modules = {}, rpc = async () => null) {
  const host = new BrowserPluginHost();
  const loader = new ExternalClientModules({ host, workspaceId: 'w', modules: { react: React, ...modules }, load, rpc,
    compile: async source => new Function('require', 'module', 'exports', source) });
  return { host, loader };
}
test('shares React and evaluates once until disabled, with workspace ownership', async () => {
  const observations = { count: 0 };
  const { host, loader } = setup(async () => `const state=require('observations');state.count++;state.react=require('react');${source}`, { observations });
  await loader.sync(enabled); await loader.sync(enabled);
  assert.equal(observations.react, React); assert.equal(observations.count, 1);
  assert.equal(host.slots.entries('settings.pages', scope).length, 1);
  assert.equal(host.slots.entries('settings.pages', { ...scope, workspaceId: 'other' }).length, 0);
  await loader.sync([]);
  assert.equal(host.slots.entries('settings.pages', scope).length, 0);
  await loader.sync(enabled); assert.equal(observations.count, 2);
  await loader.dispose(); await host.dispose();
});
test('disable then re-enable fences the previous pending source', async () => {
  const pending = deferred(); let loads = 0;
  const { host, loader } = setup(() => ++loads === 1 ? pending.promise : Promise.resolve(source));
  const first = loader.sync(enabled); await new Promise(resolve => setImmediate(resolve));
  const disable = loader.sync([]); const enable = loader.sync(enabled);
  pending.resolve(`throw Error('stale source evaluated');`);
  await Promise.all([first, disable, enable]);
  assert.deepEqual(loader.getSnapshot(), []); assert.equal(loads, 2);
  assert.deepEqual(host.activePluginIds, ['example']);
  await loader.dispose(); await host.dispose();
});
test('disposal during apply immediately removes owned resources and rejects late registration', async () => {
  const pending = deferred(); const state = { pending: pending.promise, started: deferred() };
  const { host, loader } = setup(async () => `exports.sdkVersion=2;exports.apply=async ctx=>{const s=require('state');ctx.slots.register('settings.pages',{entryId:'example',component:()=>null});s.started.resolve();await s.pending;ctx.slots.register('settings.pages',{entryId:'late',component:()=>null});};`, { state });
  const activating = loader.sync(enabled); await state.started.promise;
  const disposing = loader.dispose();
  assert.equal(host.slots.entries('settings.pages', scope).length, 0);
  pending.resolve(); await Promise.all([activating, disposing]);
  assert.deepEqual(host.activePluginIds, []); assert.deepEqual(loader.getSnapshot(), []);
  await host.dispose();
});
test('failed apply cleans up, exposes error and retries without reevaluating', async () => {
  const state = { fail: true, evaluated: 0 };
  const { host, loader } = setup(async () => `const state=require('state');state.evaluated++;exports.sdkVersion=2;exports.apply=ctx=>{ctx.slots.register('settings.pages',{entryId:'example',component:()=>null});if(state.fail)throw Error('apply failed');};`, { state });
  await loader.sync(enabled);
  assert.match(loader.getSnapshot()[0].message, /apply failed/);
  assert.equal(host.slots.entries('settings.pages', scope).length, 0);
  state.fail = false; await loader.retry('example');
  assert.deepEqual(loader.getSnapshot(), []); assert.equal(state.evaluated, 1);
  assert.equal(host.slots.entries('settings.pages', scope).length, 1);
  await loader.dispose(); await host.dispose();
});
test('RPC stays bound to package and is denied after disable or for reserved methods', async () => {
  const state = {}; const calls = [];
  const { host, loader } = setup(async () => `exports.sdkVersion=2;exports.apply=ctx=>{require('state').ctx=ctx;};`, { state }, async (...args) => { calls.push(args); return 'ok'; });
  await loader.sync(enabled);
  for (const method of ['engine.start', 'initialize', 'shutdown']) await assert.rejects(state.ctx.rpc(method, {}), /reserved/i);
  assert.equal(await state.ctx.rpc('status', { value: 1 }), 'ok');
  assert.deepEqual(calls, [['example', 'status', { value: 1 }]]);
  await loader.sync([]); await assert.rejects(state.ctx.rpc('status', {}), /disposed/i);
  await loader.dispose(); await host.dispose();
});
test('rejects incompatible exports and unknown imports and can reload after transport failure', async () => {
  for (const invalid of [`exports.sdkVersion=1;exports.apply=()=>{};`, `exports.sdkVersion=2;`, `require('unavailable');${source}`]) {
    const { host, loader } = setup(async () => invalid);
    await loader.sync(enabled); assert.equal(loader.getSnapshot().length, 1); assert.deepEqual(host.activePluginIds, []);
    await loader.dispose(); await host.dispose();
  }
  let fails = true;
  const { host, loader } = setup(async () => { if (fails) throw Error('offline'); return source; });
  await loader.sync(enabled); assert.match(loader.getSnapshot()[0].message, /offline/);
  fails = false; await loader.retry('example'); assert.deepEqual(loader.getSnapshot(), []);
  await loader.dispose(); await host.dispose();
});
test('failed activation and direct host disposal revoke captured RPC contexts', async () => {
  for (const fail of [true, false]) {
    const state = { fail };
    const { host, loader } = setup(async () => `exports.sdkVersion=2;exports.apply=ctx=>{const state=require('state');state.ctx=ctx;if(state.fail)throw Error('failed');};`, { state });
    await loader.sync(enabled);
    if (!fail) await host.dispose();
    await assert.rejects(state.ctx.rpc('status'), /disposed/i);
    await loader.dispose(); await host.dispose();
  }
});
test('disposal while compiling never evaluates the returned module', async () => {
  const pending = deferred(); const started = deferred(); const host = new BrowserPluginHost(); let evaluated = false;
  const loader = new ExternalClientModules({ host, workspaceId: 'w', modules: {}, load: async () => source,
    compile: () => { started.resolve(); return pending.promise; }, rpc: async () => null });
  const activating = loader.sync(enabled); await started.promise;
  const disposing = loader.dispose(); pending.resolve(() => { evaluated = true; });
  await Promise.all([activating, disposing]);
  assert.equal(evaluated, false); assert.deepEqual(host.activePluginIds, []);
  await host.dispose();
});
