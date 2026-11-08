import test from 'node:test';
import assert from 'node:assert/strict';
import { BrowserPluginHost, SlotRegistry, ServiceRegistry } from '../packages/plugin-sdk/src/index.ts';
const root = { kind: 'root' };
const session = { kind: 'session', workspaceId: 'w', sessionId: 's' };
const component = () => null;
const entry = (ownerPluginId, entryId = ownerPluginId) => ({ ownerPluginId, entryId, component });
test('composer menu contributions are ordered, scoped and disposed with their plugin', async () => {
  const host = new BrowserPluginHost();
  const scope={kind:'workspace',workspaceId:'w'};
  await host.activate([{id:'files',apply(ctx){ctx.scope(scope).slots.register('composer.menu',{entryId:'file',component});}}]);
  assert.equal(host.slots.entries('composer.menu',scope)[0].entryId,'file');
  assert.equal(host.slots.entries('composer.menu',{...scope,workspaceId:'other'}).length,0);
  await host.disable('files');
  assert.equal(host.slots.entries('composer.menu',scope).length,0);
});
test('slots reject unknown names, duplicate single providers, and scope mismatch', () => {
  const slots = new SlotRegistry();
  assert.throws(() => slots.register('bogus', root, entry('a')), /Unknown slot/);
  slots.register('root.layout', root, entry('a'));
  assert.throws(() => slots.register('root.layout', root, entry('b')), /Conflict/);
  assert.throws(() => slots.register('composer.actions', root, entry('a')), /scope/);
  assert.throws(() => slots.register('composer.actions', { ...session, sessionId: '' }, entry('a')), /scope/);
});
test('explicit replacements restore previous provider on disposal; list order is deterministic', () => {
  const slots = new SlotRegistry({ 'root.layout': 'b' });
  slots.register('root.layout', root, entry('a'));
  const replacement = slots.register('root.layout', root, entry('b'));
  assert.equal(slots.entries('root.layout', root)[0].ownerPluginId, 'b');
  replacement.dispose();
  assert.equal(slots.entries('root.layout', root)[0].ownerPluginId, 'a');
  slots.register('composer.actions', session, entry('z'));
  slots.register('composer.actions', session, entry('a'));
  assert.deepEqual(slots.entries('composer.actions', session).map(x => x.entryId), ['a', 'z']);
  assert.equal(slots.entries('composer.actions', { ...session, sessionId: 'other' }).length, 0);
});
test('services validate version and single providers with disposable restoration', () => {
  const services = new ServiceRegistry();
  const key = { name: 'example', version: 1 };
  const d = services.provide(key, root, 'a', 42);
  assert.equal(services.get(key, root), 42);
  assert.throws(() => services.provide(key, root, 'b', 43), /Conflict/);
  assert.throws(() => services.get({ ...key, version: 2 }, root), /Missing service/);
  d.dispose();
  assert.throws(() => services.get(key, root), /Missing service/);
});
test('disable disposes child scopes, services and listeners and permits clean re-enable', async () => {
  const host = new BrowserPluginHost();
  const target = new EventTarget(); let calls = 0;
  const plugin = { id: 'test', apply(ctx) {
    ctx.slots.register('root.layout', entry('test'));
    const child = ctx.scope(session);
    child.slots.register('composer.actions', entry('test'));
    child.listen(target, 'ping', () => calls++);
    ctx.services.provide({ name: 'example', version: 1 }, 42);
  }};
  for (let i = 0; i < 2; i++) {
    await host.activate([plugin]); target.dispatchEvent(new Event('ping'));
    await host.disable('test'); target.dispatchEvent(new Event('ping'));
    assert.equal(calls, i + 1);
    assert.equal(host.slots.entries('composer.actions', session).length, 0);
    assert.equal(host.slots.entries('root.layout', root).length, 0);
  }
  await host.dispose();
});
test('activation dependency validation and rollback leave preexisting plugins active', async () => {
  const host = new BrowserPluginHost();
  await host.activate([{ id: 'old', apply() {} }]);
  let cleaned = 0;
  await assert.rejects(host.activate([
    { id: 'dependency', apply(ctx) { ctx.effect(() => cleaned++); } },
    { id: 'failure', requires: ['dependency'], apply(ctx) { ctx.slots.register('root.layout', entry('failure')); throw Error('failed'); } },
  ]), /failed/);
  assert.equal(cleaned, 1);
  assert.deepEqual(host.activePluginIds, ['old']);
  assert.equal(host.slots.entries('root.layout', root).length, 0);
  await assert.rejects(host.activate([{ id: 'missing', requires: ['nope'], apply() {} }]), /Missing dependency/);
  await assert.rejects(host.activate([{ id: 'a', requires: ['b'], apply() {} }, { id: 'b', requires: ['a'], apply() {} }]), /cycle/);
  await host.dispose();
});
test('keyed replacement conflicts are explicit and cleanup restores original entry', () => {
  const workspace = { kind: 'workspace', workspaceId: 'w' };
  const slots = new SlotRegistry({ 'settings.pages': { general: 'replacement' } });
  slots.register('settings.pages', workspace, entry('original', 'general'));
  const d = slots.register('settings.pages', workspace, entry('replacement', 'general'));
  assert.equal(slots.entries('settings.pages', workspace)[0].ownerPluginId, 'replacement');
  assert.throws(() => slots.register('settings.pages', workspace, entry('third', 'general')), /Conflict/);
  d.dispose();
  assert.equal(slots.entries('settings.pages', workspace)[0].ownerPluginId, 'original');
});
test('dependencies activate first and cannot be disabled beneath consumers', async () => {
  const host = new BrowserPluginHost(); const order = [];
  await host.activate([
    { id: 'a-consumer', requires: ['z-provider'], apply() { order.push('consumer'); } },
    { id: 'z-provider', apply() { order.push('provider'); } },
  ]);
  assert.deepEqual(order, ['provider', 'consumer']);
  await assert.rejects(host.disable('z-provider'), /Active dependents/);
  await host.disable('a-consumer'); await host.disable('z-provider'); await host.dispose();
});
test('owned subscriptions and resources disappear even if a cleanup throws', async () => {
  const host = new BrowserPluginHost(); let notifications = 0;
  await host.activate([{ id: 'owner', apply(ctx) {
    ctx.slots.subscribe(() => notifications++);
    ctx.slots.register('root.layout', entry('owner'));
    ctx.services.provide({ name: 'resource', version: 1 }, 'value');
    ctx.effect(() => { throw Error('cleanup error'); });
  }}]);
  await assert.rejects(host.disable('owner'), /cleanup/);
  assert.equal(host.slots.entries('root.layout', root).length, 0);
  assert.throws(() => host.services.get({ name: 'resource', version: 1 }, root), /Missing service/);
  const previous = notifications;
  host.slots.register('root.layout', root, entry('other'));
  assert.equal(notifications, previous);
  await host.dispose();
});
