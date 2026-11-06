import { scopeKey } from './lifecycle.ts';
import type { Disposable, Scope } from './lifecycle.ts';
export interface ServiceKey<T> { name: string; version: number; readonly __type?: T }
export class ServiceRegistry {
  private records = new Map<string, { owner: string; value: unknown }>();
  private key<T>(key: ServiceKey<T>, scope: Scope): string {
    if (!key.name || !Number.isInteger(key.version) || key.version < 1) throw Error('Invalid service contract');
    return JSON.stringify([scopeKey(scope), key.name, key.version]);
  }
  provide<T>(key: ServiceKey<T>, scope: Scope, owner: string, value: T): Disposable {
    const id = this.key(key, scope); if (!owner) throw Error('Missing service owner');
    if (this.records.has(id)) throw Error(`Conflict in service ${key.name}`);
    const record = { owner, value }; this.records.set(id, record);
    return { dispose: () => { if (this.records.get(id) === record) this.records.delete(id); } };
  }
  get<T>(key: ServiceKey<T>, scope: Scope): T {
    const record = this.records.get(this.key(key, scope)); if (!record) throw Error(`Missing service: ${key.name}@${key.version}`); return record.value as T;
  }
}
