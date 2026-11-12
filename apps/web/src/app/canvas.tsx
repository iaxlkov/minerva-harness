import React, {useRef, useState, useSyncExternalStore} from 'react';
import {Plus, X} from 'lucide-react';
import {DropdownMenu, DropdownMenuContent, DropdownMenuTrigger} from '../components/ui/dropdown-menu';
import {Button} from '../components/ui/button';
import {UiSlot, useUiPlugins, workspaceScope} from './plugins';
import './canvas.css';

/**
 * Side canvas next to the chat. Plugins add tabs two ways: statically through the keyed
 * `shell.canvas.tabs` slot, or at runtime with `canvas.show()` for any content (markup, widgets, frames).
 */
export type CanvasTabProps = {sessionId: string; placement: 'right' | 'bottom'};
export type CanvasTab = {id: string; title: string; icon?: React.ReactNode; render: (props: CanvasTabProps) => React.ReactNode; onClose?: () => void};
type State = {open: boolean; tab: string; dynamic: readonly CanvasTab[]};
const listeners = new Set<() => void>();
const stored = (() => { try { return JSON.parse(localStorage.getItem('minerva.canvas') ?? 'null'); } catch { return null; } })();
let state: State = {open: !!stored?.open, tab: typeof stored?.tab === 'string' ? stored.tab : '', dynamic: []};
function publish(next: State) {
  state = next;
  // Runtime tabs hold render functions, so only the panel position survives a reload.
  try { localStorage.setItem('minerva.canvas', JSON.stringify({open: next.open, tab: next.tab})); } catch { /* memory only */ }
  for (const listener of listeners) listener();
}
export const canvas = {
  subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  getSnapshot: () => state,
  open: (tab: string) => publish({...state, open: true, tab}),
  close: () => publish({...state, open: false}),
  /** Opens `tab`, or closes the canvas when that tab is already showing. */
  toggle: (tab: string) => publish(state.open && state.tab === tab ? {...state, open: false} : {...state, open: true, tab}),
  /** Adds or replaces a runtime tab and shows it. */
  show(tab: CanvasTab) { publish({...state, open: true, tab: tab.id, dynamic: [...state.dynamic.filter(item => item.id !== tab.id), tab]}); },
  closeTab(id: string) {
    state.dynamic.find(item => item.id === id)?.onClose?.();
    const dynamic = state.dynamic.filter(item => item.id !== id);
    publish({...state, dynamic, tab: state.tab === id ? dynamic.at(-1)?.id ?? '' : state.tab});
  },
};
export function useCanvas() { return useSyncExternalStore(canvas.subscribe, canvas.getSnapshot); }

function readSize(key: string, fallback: number) { try { const v = Number(localStorage.getItem(key)); return v > 0 ? v : fallback; } catch { return fallback; } }

export function Canvas({sessionId, placement, section, onNavigate}: {sessionId: string; placement: 'right' | 'bottom'; section: string; onNavigate: (section: string) => void}) {
  const host = useUiPlugins();
  useSyncExternalStore(callback => { const sub = host.slots.subscribe(callback); return () => sub.dispose(); }, host.slots.getSnapshot);
  const {open, tab, dynamic} = useCanvas();
  const dock = useRef<HTMLElement>(null);
  const sizeKey = `minerva.canvas.${placement}`;
  const [size, setSize] = useState(() => readSize(sizeKey, placement === 'right' ? 460 : 280));
  const tabs = host.slots.entries('shell.canvas.tabs', workspaceScope);
  const actions = host.slots.entries('shell.canvas.actions', workspaceScope);
  const ids = [...tabs.map(entry => entry.entryId), ...dynamic.map(item => item.id)];
  if (!open || !ids.length) return null;
  const active = ids.includes(tab) ? tab : ids[0];
  const runtime = dynamic.find(item => item.id === active);
  const props = {section, onNavigate, sessionId, placement};
  const resize = (next: number) => {
    const total = placement === 'right' ? dock.current?.parentElement?.clientWidth ?? 1200 : dock.current?.parentElement?.clientHeight ?? 800;
    const value = Math.round(Math.max(placement === 'right' ? 280 : 120, Math.min(total - (placement === 'right' ? 360 : 160), next)));
    setSize(value);
    try { localStorage.setItem(sizeKey, String(value)); } catch { /* memory only */ }
  };
  return <section ref={dock} className={`shell-dock shell-canvas dock-${placement}`} style={placement === 'right' ? {width: size} : {height: size}} aria-label="Боковая панель">
    <div className="canvas-resize" role="separator" aria-orientation={placement === 'right' ? 'vertical' : 'horizontal'} aria-label="Размер панели" aria-valuenow={size} tabIndex={0}
      onPointerDown={event => event.currentTarget.setPointerCapture(event.pointerId)}
      onPointerMove={event => {
        if (!event.currentTarget.hasPointerCapture(event.pointerId) || !dock.current) return;
        const rect = dock.current.getBoundingClientRect();
        resize(placement === 'right' ? rect.right - event.clientX : rect.bottom - event.clientY);
      }}
      onKeyDown={event => {
        const grow = placement === 'right' ? 'ArrowLeft' : 'ArrowUp', shrink = placement === 'right' ? 'ArrowRight' : 'ArrowDown';
        const step = event.shiftKey ? 64 : 16;
        if (event.key === grow) resize(size + step); else if (event.key === shrink) resize(size - step); else return;
        event.preventDefault();
      }}/>
    <header className="canvas-tabs">
      <div role="tablist" aria-label="Вкладки панели" className="canvas-tablist">
        {tabs.map(entry => <button key={entry.entryId} role="tab" type="button" id={`canvas-tab-${entry.entryId}`} aria-selected={entry.entryId === active} aria-controls="canvas-body" className="canvas-tab" onClick={() => canvas.open(entry.entryId)}>
          <UiSlot slot="shell.canvas.tabs" scope={workspaceScope} entryId={entry.entryId} componentProps={{...props, part: 'tab', selected: entry.entryId === active}}/>
        </button>)}
        {dynamic.map(item => <div key={item.id} className="canvas-tab-group">
          <button role="tab" type="button" id={`canvas-tab-${item.id}`} aria-selected={item.id === active} aria-controls="canvas-body" className="canvas-tab" onClick={() => canvas.open(item.id)}>{item.icon}{item.title}</button>
          <button type="button" className="canvas-tab-close" aria-label={`Закрыть вкладку ${item.title}`} onClick={() => canvas.closeTab(item.id)}><X/></button>
        </div>)}
      </div>
      {actions.length > 0 && <DropdownMenu>
        <DropdownMenuTrigger asChild><Button variant="ghost" size="icon-sm" className="canvas-close" aria-label="Открыть в панели" title="Открыть в панели"><Plus/></Button></DropdownMenuTrigger>
        <DropdownMenuContent align="end"><UiSlot slot="shell.canvas.actions" scope={workspaceScope} componentProps={props}/></DropdownMenuContent>
      </DropdownMenu>}
      <Button variant="ghost" size="icon-sm" className="canvas-close" aria-label="Закрыть панель" title="Закрыть панель" onClick={canvas.close}><X/></Button>
    </header>
    <div id="canvas-body" role="tabpanel" aria-labelledby={`canvas-tab-${active}`} className="canvas-body">
      {runtime ? <React.Fragment key={`${active}/${sessionId}`}>{runtime.render({sessionId, placement})}</React.Fragment>
        : <UiSlot key={`${active}/${sessionId}`} slot="shell.canvas.tabs" scope={workspaceScope} entryId={active} componentProps={{...props, part: 'body', selected: true}}/>}
    </div>
  </section>;
}
