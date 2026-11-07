import React, { useSyncExternalStore } from 'react';
import type { ExternalClientModules } from '../../../../packages/plugin-sdk/src/client-modules.ts';
import { Button } from '../components/ui/button';

export function ExternalPluginErrors({ loader }: { loader: ExternalClientModules }) {
  const errors = useSyncExternalStore(loader.subscribe, loader.getSnapshot, loader.getSnapshot);
  return <>{errors.map(error => <section className="session-error" role="alert" key={error.id}>
    <span>Plugin {error.id}: {error.message}</span>
    <Button variant="outline" onClick={() => { void loader.retry(error.id); }}>Retry</Button>
  </section>)}</>;
}
