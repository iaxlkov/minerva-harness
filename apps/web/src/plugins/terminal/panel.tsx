import React, {useEffect, useRef, useState, useSyncExternalStore} from 'react';
import {Terminal} from '@xterm/xterm';
import {FitAddon} from '@xterm/addon-fit';
import '@xterm/xterm/css/xterm.css';
import {Plus, RotateCw} from 'lucide-react';
import {invoke} from '../../client';
import {models} from '../../app/models';
import {Button} from '../../components/ui/button';
import {onTerminalReady, pending, terminals} from './store';
import './terminal.css';

/** One shell per (session, slot): the main tab uses no slot, tabs opened with "+" get their own. */
export default function TerminalPanel({sessionId, slot = '', onNew}: {sessionId: string; slot?: string; onNew?: () => void}) {
  const key = slot ? `${sessionId}#${slot}` : sessionId;
  const host = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<{state: 'running' | 'exited' | 'error'; error?: string}>({state: 'running'});
  const [generation, setGeneration] = useState(0);
  const workspaces = useSyncExternalStore(models.workspaces.subscribe, models.workspaces.getSnapshot, models.workspaces.getSnapshot);
  const workspace = workspaces.find(w => w.sessionIds.includes(sessionId));

  useEffect(() => {
    const element = host.current!;
    const css = getComputedStyle(element);
    const color = (name: string, fallback: string) => css.getPropertyValue(name).trim() || fallback;
    const term = new Terminal({
      fontFamily: 'ui-monospace, "JetBrains Mono", "Cascadia Mono", Menlo, Consolas, monospace',
      fontSize: 12, cursorBlink: true, scrollback: 5000,
      theme: {background: color('--background', '#1c1b1a'), foreground: color('--foreground', '#e9e6e1'), cursor: color('--primary', '#d0a087'), selectionBackground: '#ffffff33'},
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(element);
    fit.fit();
    let alive = true, cursor = 0, timer: ReturnType<typeof setTimeout> | undefined;
    let id = terminals.get(key);
    const fail = (cause: unknown) => { if (alive) setStatus({state: 'error', error: String(cause).replace(/^MinervaError: /, '')}); };
    // Poll fast while output flows, slowly when idle; the backend keeps output, so a remount replays from 0.
    const poll = async () => {
      if (!alive || !id) return;
      let delay = 200;
      try {
        const chunk = await invoke('mh-terminal.read', {sessionId, id, cursor});
        if (!alive) return;
        cursor = chunk.cursor;
        if (chunk.data) { term.write(Uint8Array.from(atob(chunk.data), c => c.charCodeAt(0))); delay = 30; }
        if (chunk.exited) { setStatus({state: 'exited'}); return; }
      } catch (cause) { terminals.delete(key); fail(cause); return; }
      timer = setTimeout(() => void poll(), delay);
    };
    // Commands sent from chat ("Выполнить") wait here until this session's shell exists.
    const flush = () => {
      const queued = pending.get(key);
      if (!id || !queued?.length) return;
      pending.delete(key);
      for (const data of queued) void invoke('mh-terminal.write', {sessionId, id, data}).catch(fail);
    };
    const stopReady = onTerminalReady(key, flush);
    void (async () => {
      try {
        // A remembered id may belong to a terminal that ended with the previous app run: open a fresh one then.
        if (id) await invoke('mh-terminal.resize', {sessionId, id, cols: term.cols, rows: term.rows}).catch(() => { id = undefined; terminals.delete(key); });
        if (!id) { id = await invoke('mh-terminal.open', {sessionId, cols: term.cols, rows: term.rows}); terminals.set(key, id); }
        flush();
        void poll();
      } catch (cause) { fail(cause); }
    })();
    const input = term.onData(data => { if (id) void invoke('mh-terminal.write', {sessionId, id, data}).catch(fail); });
    const observer = new ResizeObserver(() => {
      if (!element.clientWidth || !element.clientHeight) return;
      fit.fit();
      if (id) void invoke('mh-terminal.resize', {sessionId, id, cols: term.cols, rows: term.rows}).catch(() => {});
    });
    observer.observe(element);
    setStatus({state: 'running'});
    term.focus();
    return () => { alive = false; clearTimeout(timer); stopReady(); observer.disconnect(); input.dispose(); term.dispose(); };
  }, [sessionId, key, generation]);

  async function restart() {
    const id = terminals.get(key);
    terminals.delete(key);
    if (id) await invoke('mh-terminal.close', {sessionId, id}).catch(() => {});
    setGeneration(value => value + 1);
  }
  return <div className="terminal-panel">
    <div className="terminal-toolbar">
      {workspace && <span className="terminal-cwd" title={workspace.path ?? undefined}>{workspace.title}</span>}
      <div className="terminal-toolbar-actions">
        {onNew && <Button variant="ghost" size="icon-sm" aria-label="Новый терминал" title="Новый терминал" onClick={onNew}><Plus/></Button>}
        <Button variant="ghost" size="icon-sm" aria-label="Перезапустить терминал" title="Перезапустить" onClick={() => void restart()}><RotateCw/></Button>
      </div>
    </div>
    {status.state !== 'running' && <div className="terminal-note" role={status.state === 'error' ? 'alert' : 'status'}>
      <span>{status.state === 'exited' ? 'Процесс оболочки завершён.' : status.error}</span>
      <Button variant="outline" size="sm" onClick={() => void restart()}>Перезапустить</Button>
    </div>}
    <div className="terminal-host" ref={host}/>
  </div>;
}
