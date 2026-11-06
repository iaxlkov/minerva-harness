import { Component, createElement, useSyncExternalStore } from 'react';
import type { ComponentType, ReactNode } from 'react';
import type { Scope } from './lifecycle.ts';
import type { SlotRegistry } from './slots.ts';
import type { SlotEntry, SlotName } from './slots.types.ts';
interface BoundaryProps { ownerPluginId: string; children: ReactNode; onOpenSettings: () => void; onDisable: (pluginId: string) => void }
class PluginBoundary extends Component<BoundaryProps, { failed: boolean }> {
  state = { failed: false };
  static getDerivedStateFromError() { return { failed: true }; }
  render() {
    if (!this.state.failed) return this.props.children;
    return <section role="alert"><p>Plugin {this.props.ownerPluginId} could not display this view.</p><button onClick={this.props.onOpenSettings}>Open settings</button><button onClick={() => this.props.onDisable(this.props.ownerPluginId)}>Disable plugin</button></section>;
  }
}
export interface SlotRendererProps {
  registry: SlotRegistry;
  slot: SlotName;
  scope: Scope;
  entryId?: string;
  componentProps?: Readonly<Record<string, unknown>>;
  fallback?: ReactNode;
  onOpenSettings: () => void;
  onDisable: (pluginId: string) => void;
}
/** Presentation receives only adapter-supplied props, never a lifecycle context. */
export function SlotRenderer({ registry, slot, scope, entryId, componentProps = {}, fallback = null, onOpenSettings, onDisable }: SlotRendererProps) {
  useSyncExternalStore(callback => { const subscription = registry.subscribe(callback); return () => subscription.dispose(); }, registry.getSnapshot, registry.getSnapshot);
  const entries = registry.entries(slot, scope).filter(entry => entryId === undefined || entry.entryId === entryId);
  if (!entries.length) return fallback;
  return entries.map((entry: SlotEntry) => <PluginBoundary key={`${entry.ownerPluginId}:${entry.entryId}`} ownerPluginId={entry.ownerPluginId} onOpenSettings={onOpenSettings} onDisable={onDisable}>{createElement(entry.component as ComponentType<Record<string, unknown>>, componentProps)}</PluginBoundary>);
}
