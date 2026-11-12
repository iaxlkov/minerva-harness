import React, {Suspense, lazy} from 'react';
import {TerminalSquare} from 'lucide-react';
import type {BrowserPlugin, Scope} from '../../../../../packages/plugin-sdk/src/index';
import type {AppSlotProps} from '../../app/plugins';
import {canvas, useCanvas} from '../../app/canvas';
import {Button} from '../../components/ui/button';
import {Tooltip, TooltipContent, TooltipTrigger} from '../../components/ui/tooltip';
import {CommandBlock} from './CommandBlock';
import {invoke} from '../../client';
import {terminals} from './store';
const TerminalPanel = lazy(() => import('./panel'));
const TAB = 'mh-terminal';
let next = 2;
/** Opens another shell as its own canvas tab; closing the tab ends that shell in every session that used it. */
function openNewTerminal() {
  const slot = String(next++);
  canvas.show({
    id: `${TAB}:${slot}`, title: `Терминал ${slot}`, icon: <TerminalSquare aria-hidden="true"/>,
    render: ({sessionId}) => <Suspense fallback={<p className="terminal-note" role="status">Загрузка терминала…</p>}><TerminalPanel sessionId={sessionId} slot={slot} onNew={openNewTerminal}/></Suspense>,
    onClose: () => { for (const [key, id] of terminals.slot(slot)) void invoke('mh-terminal.close', {sessionId: key.split('#')[0], id}).catch(() => {}); },
  });
}

function TitlebarButton() {
  const {open, tab} = useCanvas();
  return <Tooltip><TooltipTrigger asChild><Button variant="ghost" size="icon-lg" className="titlebar-button" aria-label="Терминал" aria-pressed={open && tab === TAB} onClick={() => canvas.toggle(TAB)}><TerminalSquare/></Button></TooltipTrigger><TooltipContent className="oe-studio">Терминал</TooltipContent></Tooltip>;
}
function Tab(props: AppSlotProps['shell.canvas.tabs']) {
  if (props.part === 'tab') return <><TerminalSquare aria-hidden="true"/>Терминал</>;
  return <Suspense fallback={<p className="terminal-note" role="status">Загрузка терминала…</p>}><TerminalPanel sessionId={props.sessionId} onNew={openNewTerminal}/></Suspense>;
}
export function terminalPlugin(scope: Scope): BrowserPlugin {
  return {id: 'mh-terminal-ui', requires: ['minerva-ui'], apply(ctx) {
    const scoped = ctx.scope(scope);
    scoped.slots.register('shell.titlebar.end', {entryId: TAB, order: 100, component: TitlebarButton});
    scoped.slots.register('shell.canvas.tabs', {entryId: TAB, component: Tab});
    scoped.slots.register('message.code', {entryId: 'shell', component: CommandBlock});
  }};
}
