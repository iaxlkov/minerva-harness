import { useEffect, useSyncExternalStore } from 'react';

/** Keep-awake preferences: a global switch plus per-session opt-ins, held while those sessions run. */
type State = { global: boolean; sessions: string[]; error: string };
export const keepAwakeAvailable = '__TAURI_INTERNALS__' in globalThis;
const listeners = new Set<() => void>();
const running = new Set<string>();
let held = false;
function load(): State {
  try {
    const sessions = JSON.parse(localStorage.getItem('minerva.keepAwake.sessions') ?? '[]');
    return { global: localStorage.getItem('minerva.keepAwake') === '1', sessions: Array.isArray(sessions) ? sessions.filter(id => typeof id === 'string') : [], error: '' };
  } catch { return { global: false, sessions: [], error: '' }; }
}
let state = load();
function publish(next: State) {
  state = next;
  try {
    localStorage.setItem('minerva.keepAwake', next.global ? '1' : '0');
    localStorage.setItem('minerva.keepAwake.sessions', JSON.stringify(next.sessions));
  } catch { /* Preferences stay in memory when storage is unavailable. */ }
  for (const listener of listeners) listener();
  void sync();
}
async function sync() {
  const want = [...running].some(id => state.global || state.sessions.includes(id));
  if (!keepAwakeAvailable || want === held) return;
  held = want;
  try {
    const { invoke } = await import('@tauri-apps/api/core');
    await invoke('set_keep_awake', { enabled: want });
    if (state.error) { state = { ...state, error: '' }; for (const listener of listeners) listener(); }
  } catch (cause) {
    held = !want;
    state = { ...state, error: String(cause) };
    for (const listener of listeners) listener();
  }
}
export const keepAwake = {
  subscribe(listener: () => void) { listeners.add(listener); return () => { listeners.delete(listener); }; },
  getSnapshot: () => state,
  setGlobal: (global: boolean) => publish({ ...state, global }),
  setSession: (id: string, on: boolean) => publish({ ...state, sessions: on ? [...new Set([...state.sessions, id])] : state.sessions.filter(item => item !== id) }),
};
export function useKeepAwake() { return useSyncExternalStore(keepAwake.subscribe, keepAwake.getSnapshot); }
// shortcut: only sessions the window currently shows report runs; move this into the runtime when background runs need it.
export function useRunningSession(sessionId: string, isRunning: boolean) {
  useEffect(() => {
    if (!isRunning) return;
    running.add(sessionId); void sync();
    return () => { running.delete(sessionId); void sync(); };
  }, [sessionId, isRunning]);
}
