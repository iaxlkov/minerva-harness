import { Component, createElement, useSyncExternalStore } from 'react';
import type { ComponentType, ReactNode } from 'react';
import type { Scope } from './lifecycle.ts';
import type { SlotRegistry } from './slots.ts';
import type { SlotEntry, SlotName } from './slots.types.ts';
export interface PluginErrorProps { pluginId: string; error: string; actionError: string; onOpenSettings: () => void; onDisable: () => void }
interface BoundaryProps { ownerPluginId: string; children: ReactNode; onOpenSettings: () => void; onDisable: (pluginId: string) => void | Promise<void>; renderError?: (props: PluginErrorProps) => ReactNode }
class PluginBoundary extends Component<BoundaryProps, { failed: boolean; error: string; actionError: string }> {
  state = { failed: false, error: '', actionError: '' };
  static getDerivedStateFromError(error: unknown) { return { failed: true, error: String(error) }; }
  disable = async () => {
    try { await this.props.onDisable(this.props.ownerPluginId); }
    catch(error) { this.setState({actionError:String(error)}); }
  };
  render() {
    if (!this.state.failed) return this.props.children;
    if (this.props.renderError) return this.props.renderError({pluginId:this.props.ownerPluginId,error:this.state.error,actionError:this.state.actionError,onOpenSettings:this.props.onOpenSettings,onDisable:this.disable});
    return <section role="alert"><p>Plugin {this.props.ownerPluginId} could not display this view.</p><p>{this.state.error}</p>{this.state.actionError&&<p>{this.state.actionError}</p>}<button onClick={this.props.onOpenSettings}>Open settings</button><button onClick={this.disable}>Disable plugin</button></section>;
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
  onDisable: (pluginId: string) => void | Promise<void>;
  renderError?: (props: PluginErrorProps) => ReactNode;
}
/** Presentation receives only adapter-supplied props, never a lifecycle context. */
export function SlotRenderer({ registry, slot, scope, entryId, componentProps = {}, fallback = null, onOpenSettings, onDisable, renderError }: SlotRendererProps) {
  useSyncExternalStore(callback => { const subscription = registry.subscribe(callback); return () => subscription.dispose(); }, registry.getSnapshot, registry.getSnapshot);
  const entries = registry.entries(slot, scope).filter(entry => entryId === undefined || entry.entryId === entryId);
  if (!entries.length) return fallback;
  return entries.map((entry: SlotEntry) => <PluginBoundary key={`${entry.ownerPluginId}:${entry.entryId}`} ownerPluginId={entry.ownerPluginId} onOpenSettings={onOpenSettings} onDisable={onDisable} renderError={renderError}>{createElement(entry.component as ComponentType<Record<string, unknown>>, componentProps)}</PluginBoundary>);
}
