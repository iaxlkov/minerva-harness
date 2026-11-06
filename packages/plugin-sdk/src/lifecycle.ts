export type Disposable = { dispose(): void };
export type Scope = { kind: 'root' } | { kind: 'workspace'; workspaceId: string } | { kind: 'session'; workspaceId: string; sessionId: string };
export function scopeKey(scope: Scope): string {
  if (scope.kind !== 'root' && !scope.workspaceId || scope.kind === 'session' && !scope.sessionId) throw Error('Invalid scope');
  return JSON.stringify(scope.kind === 'root' ? ['root'] : scope.kind === 'workspace' ? ['workspace', scope.workspaceId] : ['session', scope.workspaceId, scope.sessionId]);
}
export class Ownership implements Disposable {
  private cleanups = new Set<Disposable>();
  private closed = false;
  own(item: Disposable): Disposable {
    if (this.closed) { item.dispose(); throw Error('Disposed plugin context'); }
    let active = true;
    const wrapped = { dispose: () => { if (!active) return; active = false; this.cleanups.delete(wrapped); item.dispose(); } };
    this.cleanups.add(wrapped); return wrapped;
  }
  dispose(): void {
    if (this.closed) return; this.closed = true;
    const errors: unknown[] = [];
    for (const item of [...this.cleanups].reverse()) { try { item.dispose(); } catch (error) { errors.push(error); } }
    if (errors.length) throw new AggregateError(errors, 'Plugin cleanup failed');
  }
}
