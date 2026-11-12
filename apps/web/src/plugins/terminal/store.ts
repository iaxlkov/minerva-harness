/** Backend terminal id per chat session; sessionStorage lets a reloaded window reattach instead of leaking shells. */
function loadIds(): Record<string, string> { try { return JSON.parse(sessionStorage.getItem('minerva.terminal.ids') ?? '{}') ?? {}; } catch { return {}; } }
const ids = loadIds();
function saveIds() { try { sessionStorage.setItem('minerva.terminal.ids', JSON.stringify(ids)); } catch { /* memory only */ } }
export const terminals = {
  get: (sessionId: string): string | undefined => typeof ids[sessionId] === 'string' ? ids[sessionId] : undefined,
  set(sessionId: string, id: string) { ids[sessionId] = id; saveIds(); },
  delete(sessionId: string) { delete ids[sessionId]; saveIds(); },
  /** Forgets and returns every terminal opened for `slot`, as [key, id] pairs. */
  slot(slot: string): [string, string][] {
    const found = Object.entries(ids).filter(([key]) => key.endsWith(`#${slot}`));
    for (const [key] of found) delete ids[key];
    saveIds();
    return found;
  },
};
/** Input waiting for a session's terminal to open; the panel flushes it once the shell is up. */
export const pending = new Map<string, string[]>();
const flushers = new Map<string, () => void>();
export function onTerminalReady(sessionId: string, flush: () => void) { flushers.set(sessionId, flush); return () => { if (flushers.get(sessionId) === flush) flushers.delete(sessionId); }; }
/** Queues `input` for the session's terminal and wakes a mounted panel to send it. */
export function sendToTerminal(sessionId: string, input: string) {
  pending.set(sessionId, [...(pending.get(sessionId) ?? []), input]);
  flushers.get(sessionId)?.();
}
