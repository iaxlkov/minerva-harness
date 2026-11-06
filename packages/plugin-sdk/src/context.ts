import { Context } from 'cordis';
import type { Fiber } from 'cordis';
import { Ownership, scopeKey } from './lifecycle.ts';
import type { Disposable, Scope } from './lifecycle.ts';
import { SlotRegistry } from './slots.ts';
import type { SlotEntry, SlotName, SlotSelection } from './slots.types.ts';
import { ServiceRegistry } from './services.ts';
import type { ServiceKey } from './services.ts';
export interface BrowserPlugin { id: string; requires?: readonly string[]; apply(ctx: BrowserPluginContext): void | Disposable | Promise<void | Disposable> }
export class BrowserPluginContext {
  readonly ownerPluginId: string;
  readonly currentScope: Scope;
  private ownership: Ownership;
  private slotRegistry: SlotRegistry;
  private serviceRegistry: ServiceRegistry;
  constructor(owner: string, scope: Scope, slots: SlotRegistry, services: ServiceRegistry, ownership: Ownership) {
    this.ownerPluginId = owner; this.currentScope = Object.freeze({ ...scope }); this.slotRegistry = slots; this.serviceRegistry = services; this.ownership = ownership;
  }
  get slots() { return {
    register: (slot: SlotName, entry: Omit<SlotEntry, 'ownerPluginId'> & { ownerPluginId?: string }) => {
      if (entry.ownerPluginId && entry.ownerPluginId !== this.ownerPluginId) throw Error('Plugin owner mismatch');
      return this.ownership.own(this.slotRegistry.register(slot, this.currentScope, { ...entry, ownerPluginId: this.ownerPluginId }));
    },
    subscribe: (listener: () => void) => this.ownership.own(this.slotRegistry.subscribe(listener)),
  }; }
  get services() { return {
    provide: <T>(key: ServiceKey<T>, value: T) => this.ownership.own(this.serviceRegistry.provide(key, this.currentScope, this.ownerPluginId, value)),
    get: <T>(key: ServiceKey<T>) => this.serviceRegistry.get(key, this.currentScope),
  }; }
  scope(scope: Scope): BrowserPluginContext {
    if (this.currentScope.kind === 'session' && scopeKey(this.currentScope) !== scopeKey(scope) || this.currentScope.kind === 'workspace' && (scope.kind === 'root' || scope.workspaceId !== this.currentScope.workspaceId)) throw Error('Child scope mismatch');
    const ownership = new Ownership(); this.ownership.own(ownership);
    return new BrowserPluginContext(this.ownerPluginId, scope, this.slotRegistry, this.serviceRegistry, ownership);
  }
  effect(cleanup: () => void): Disposable { return this.ownership.own({ dispose: cleanup }); }
  listen(target: EventTarget, type: string, listener: EventListener, options?: AddEventListenerOptions): Disposable {
    target.addEventListener(type, listener, options);
    return this.effect(() => target.removeEventListener(type, listener, options));
  }
}
export class BrowserPluginHost {
  readonly slots: SlotRegistry;
  readonly services = new ServiceRegistry();
  private cordis = new Context();
  private active = new Map<string, { plugin: BrowserPlugin; fiber: Fiber; ownership: Ownership }>();
  private pending: Promise<unknown> = Promise.resolve();
  private closed = false;
  constructor(selection: SlotSelection = {}) { this.slots = new SlotRegistry(selection); }
  get activePluginIds(): string[] { return [...this.active.keys()].sort(); }
  private serial<T>(action: () => Promise<T>): Promise<T> { const result = this.pending.then(action); this.pending = result.catch(() => {}); return result; }
  activate(plugins: readonly BrowserPlugin[]): Promise<void> { return this.serial(async () => {
    if (this.closed) throw Error('Disposed plugin host');
    const candidates = new Map<string, BrowserPlugin>();
    for (const plugin of plugins) { if (!plugin.id || candidates.has(plugin.id) || this.active.has(plugin.id)) throw Error(`Duplicate plugin: ${plugin.id}`); candidates.set(plugin.id, plugin); }
    const visiting = new Set<string>(), visited = new Set<string>(), ordered: BrowserPlugin[] = [];
    const visit = (id: string) => {
      if (this.active.has(id) || visited.has(id)) return;
      if (visiting.has(id)) throw Error(`Dependency cycle: ${id}`);
      const plugin = candidates.get(id); if (!plugin) throw Error(`Missing dependency: ${id}`);
      visiting.add(id); for (const dependency of [...plugin.requires ?? []].sort()) visit(dependency);
      visiting.delete(id); visited.add(id); ordered.push(plugin);
    };
    for (const id of [...candidates.keys()].sort()) visit(id);
    const staged: string[] = [];
    try {
      for (const plugin of ordered) {
        const ownership = new Ownership(); let activationError: unknown; let failed = false;
        const fiber = this.cordis.plugin(async (ctx: Context) => {
          ctx.effect(() => () => ownership.dispose());
          try {
            const result = await plugin.apply(new BrowserPluginContext(plugin.id, { kind: 'root' }, this.slots, this.services, ownership));
            if (result) ownership.own(result);
          } catch (error) { failed = true; activationError = error; }
        });
        try { await fiber; if (failed) throw activationError; }
        catch (error) { await fiber.dispose(); ownership.dispose(); throw error; }
        this.active.set(plugin.id, { plugin, fiber, ownership }); staged.push(plugin.id);
      }
    } catch (error) {
      const errors: unknown[] = [error];
      for (const id of staged.reverse()) { try { await this.remove(id); } catch (cleanup) { errors.push(cleanup); } }
      if (errors.length > 1) throw new AggregateError(errors, 'Plugin activation rollback failed'); throw error;
    }
  }); }
  private async remove(id: string): Promise<void> {
    const entry = this.active.get(id); if (!entry) return; this.active.delete(id);
    try { entry.ownership.dispose(); } finally { await entry.fiber.dispose(); }
  }
  disable(id: string): Promise<void> { return this.serial(async () => {
    const dependents = [...this.active].filter(([,entry]) => entry.plugin.requires?.includes(id)).map(([key]) => key);
    if (dependents.length) throw Error(`Active dependents: ${dependents.sort().join(', ')}`);
    await this.remove(id);
  }); }
  dispose(): Promise<void> { return this.serial(async () => {
    this.closed = true; const errors: unknown[] = [];
    for (const id of [...this.active.keys()].reverse()) { try { await this.remove(id); } catch (error) { errors.push(error); } }
    await this.cordis.fiber.dispose(); if (errors.length) throw new AggregateError(errors, 'Host cleanup failed');
  }); }
}
