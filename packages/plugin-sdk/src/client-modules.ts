import { BrowserPluginContext, BrowserPluginHost } from './context.ts';
import { Ownership } from './lifecycle.ts';
import type { Disposable } from './lifecycle.ts';

export type ExternalPluginContext = BrowserPluginContext & { rpc(method: string, params?: unknown): Promise<unknown> };
type ClientModule = { sdkVersion: 2; apply(ctx: ExternalPluginContext): void | Disposable | Promise<void | Disposable> };
type EnabledModule = { module?: ClientModule; ownership?: Ownership; failed?: boolean };
export type ClientModuleError = { id: string; message: string };
export type ClientModuleFactory = (require: (name: string) => unknown, module: { exports: unknown }, exports: unknown) => void;

/** Trusted, verified single-file CommonJS bundles share the host's module instances. */
export class ExternalClientModules {
  private options: {
    host: BrowserPluginHost; workspaceId: string; modules: Readonly<Record<string, unknown>>;
    load(id: string): Promise<string>; rpc(id: string, method: string, params: unknown): Promise<unknown>;
    compile(source: string): Promise<ClientModuleFactory>;
  };
  private enabled = new Map<string, EnabledModule>();
  private active = new Map<string, EnabledModule>();
  private pending: Promise<void> = Promise.resolve();
  private closed = false;
  private errors = new Map<string, string>();
  private snapshot: readonly ClientModuleError[] = [];
  private listeners = new Set<() => void>();
  constructor(options: ExternalClientModules['options']) { this.options = options; }
  getSnapshot = () => this.snapshot;
  subscribe = (listener: () => void) => { this.listeners.add(listener); return () => { this.listeners.delete(listener); }; };
  private publish() {
    this.snapshot = [...this.errors].map(([id, message]) => ({ id, message }));
    for (const listener of this.listeners) listener();
  }
  private report(id: string, error: unknown) { if (!this.closed) { this.errors.set(id, error instanceof Error ? error.message : String(error)); this.publish(); } }
  private current(id: string, entry: EnabledModule) { return !this.closed && this.enabled.get(id) === entry; }
  private release(id: string, entry: EnabledModule) {
    try { entry.ownership?.dispose(); } catch (error) { this.report(id, error); }
  }
  sync(plugins: readonly { id: string; enabled: boolean; origin: string; hasUi: boolean }[]): Promise<void> {
    if (this.closed) return this.pending;
    const desired = new Set(plugins.filter(p => p.enabled && p.origin === 'installed' && p.hasUi).map(p => p.id));
    for (const [id, entry] of this.enabled) if (!desired.has(id)) {
      this.enabled.delete(id); this.errors.delete(id); this.release(id, entry);
    }
    for (const id of desired) if (!this.enabled.has(id)) this.enabled.set(id, {});
    this.publish();
    return this.enqueue();
  }
  retry(id: string): Promise<void> {
    const entry = this.enabled.get(id);
    if (!entry || this.closed) return this.pending;
    entry.failed = false; this.errors.delete(id); this.publish();
    return this.enqueue();
  }
  private enqueue() {
    this.pending = this.pending.then(() => this.reconcile());
    return this.pending;
  }
  private evaluate(factory: ClientModuleFactory): ClientModule {
    const module = { exports: {} as unknown };
    const require = (name: string) => {
      if (!Object.hasOwn(this.options.modules, name)) throw Error(`Unsupported plugin import: ${name}`);
      return this.options.modules[name];
    };
    factory(require, module, module.exports);
    const exported = module.exports as Partial<ClientModule> | null;
    if (!exported || exported.sdkVersion !== 2 || typeof exported.apply !== 'function') throw Error('Plugin UI requires sdkVersion 2 and apply(ctx)');
    return exported as ClientModule;
  }
  private async reconcile() {
    const { host } = this.options;
    for (const [id, entry] of this.active) if (!this.current(id, entry)) {
      this.active.delete(id);
      try { await host.disable(id); } catch (error) { this.report(id, error); }
    }
    for (const [id, entry] of this.enabled) {
      if (!this.current(id, entry) || this.active.has(id) || entry.failed) continue;
      try {
        if (!entry.module) {
          const source = await this.options.load(id);
          if (!this.current(id, entry)) continue;
          const factory = await this.options.compile(source);
          if (!this.current(id, entry)) continue;
          entry.module = this.evaluate(factory);
        }
        if (!this.current(id, entry)) continue;
        await host.activate([{ id, apply: async root => {
          if (!this.current(id, entry)) throw Error('Disposed plugin context');
          const ownership = new Ownership(); entry.ownership = ownership;
          root.effect(() => ownership.dispose());
          const context = new BrowserPluginContext(id, { kind: 'workspace', workspaceId: this.options.workspaceId }, host.slots, host.services, ownership);
          let alive = true;
          context.effect(() => { alive = false; });
          const rpc = async (method: string, params: unknown = {}) => {
            if (!alive || !this.current(id, entry)) throw Error('Disposed plugin context');
            if (typeof method !== 'string' || !method || method.startsWith('engine.') || method === 'initialize' || method === 'shutdown') throw Error('Reserved plugin RPC method');
            return this.options.rpc(id, method, params);
          };
          const cleanup = await entry.module!.apply(Object.assign(context, { rpc }));
          if (cleanup) ownership.own(cleanup);
        } }]);
        this.active.set(id, entry);
        if (!this.current(id, entry)) { this.active.delete(id); await host.disable(id); }
        else { this.errors.delete(id); this.publish(); }
      } catch (error) {
        this.release(id, entry);
        if (this.current(id, entry)) { entry.failed = true; this.report(id, error); }
      }
    }
  }
  dispose(): Promise<void> {
    if (this.closed) return this.pending;
    this.closed = true;
    for (const [id, entry] of this.enabled) this.release(id, entry);
    this.enabled.clear(); this.errors.clear(); this.publish(); this.listeners.clear();
    return this.enqueue();
  }
}
