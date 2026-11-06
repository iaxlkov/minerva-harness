import { scopeKey } from './lifecycle.ts';
import type { Disposable, Scope } from './lifecycle.ts';
import { slotDeclarations } from './slots.types.ts';
import type { SlotEntry, SlotName, SlotSelection } from './slots.types.ts';
export class SlotRegistry {
  private records = new Map<string, SlotEntry[]>();
  private listeners = new Set<() => void>();
  private revision = 0;
  private selection: SlotSelection;
  constructor(selection: SlotSelection = {}) { this.selection = structuredClone(selection); }
  getSnapshot = (): number => this.revision;
  subscribe = (listener: () => void): Disposable => { this.listeners.add(listener); return { dispose: () => { this.listeners.delete(listener); } }; };
  private changed(): void { this.revision++; for (const listener of [...this.listeners]) { try { listener(); } catch { /* observers cannot corrupt registration */ } } }
  private key(slot: SlotName, scope: Scope): string {
    if (!Object.hasOwn(slotDeclarations, slot)) throw Error(`Unknown slot: ${slot}`);
    if (slotDeclarations[slot].scope !== scope.kind) throw Error(`Invalid scope for ${slot}`);
    return JSON.stringify([slot, scopeKey(scope)]);
  }
  register(slot: SlotName, scope: Scope, entry: SlotEntry): Disposable {
    const key = this.key(slot, scope); const kind = slotDeclarations[slot].kind;
    if (!entry.entryId || !entry.ownerPluginId || typeof entry.component !== 'function' || !Number.isFinite(entry.order ?? 0)) throw Error('Invalid slot entry');
    const records = this.records.get(key) ?? [];
    if (records.some(x => x.ownerPluginId === entry.ownerPluginId && x.entryId === entry.entryId)) throw Error('Duplicate slot entry');
    const conflicts = records.filter(x => kind === 'single' || kind === 'keyed' && x.entryId === entry.entryId);
    const configured = this.selection[slot];
    const chosen = typeof configured === 'string' ? configured : configured?.[entry.entryId];
    if (conflicts.length && (!chosen || ![...conflicts, entry].some(x => x.ownerPluginId === chosen) || conflicts.some(x => x.ownerPluginId !== chosen && entry.ownerPluginId !== chosen))) throw Error(`Conflict in ${slot}`);
    const stored = Object.freeze({ ...entry }); records.push(stored); this.records.set(key, records); this.changed();
    return { dispose: () => { const index = records.indexOf(stored); if (index < 0) return; records.splice(index, 1); if (!records.length) this.records.delete(key); this.changed(); } };
  }
  entries(slot: SlotName, scope: Scope): readonly SlotEntry[] {
    const records = this.records.get(this.key(slot, scope)) ?? []; const kind = slotDeclarations[slot].kind;
    if (kind === 'list') return [...records].sort((a,b) => (a.order ?? 0) - (b.order ?? 0) || compare(a.ownerPluginId,b.ownerPluginId) || compare(a.entryId,b.entryId));
    const groups = new Map<string, SlotEntry[]>();
    for (const entry of records) { const key = kind === 'single' ? '' : entry.entryId; groups.set(key, [...groups.get(key) ?? [], entry]); }
    return [...groups.entries()].sort(([a],[b]) => compare(a,b)).map(([key, entries]) => {
      const configured = this.selection[slot]; const chosen = typeof configured === 'string' ? configured : configured?.[key];
      return entries.find(x => x.ownerPluginId === chosen) ?? entries[0];
    });
  }
}
function compare(a: string,b: string): number { return a < b ? -1 : a > b ? 1 : 0; }
