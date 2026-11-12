import {Input} from '../components/ui/input';
import * as sidebar from '../components/ui/sidebar';
import * as resizable from '../components/ui/resizable';
import * as tooltip from '../components/ui/tooltip';
import * as React from 'react';
import * as jsxRuntime from 'react/jsx-runtime';
import * as ReactDOM from 'react-dom';
import * as ReactDOMClient from 'react-dom/client';
import { Button } from '../components/ui/button';
import * as select from '../components/ui/select';
import * as dropdown from '../components/ui/dropdown-menu';
import {ComposerMenuItem} from '../components/assistant-ui/composer-menu';
import {ContextMenu} from '../components/ContextMenu';
import { ExternalClientModules } from '../../../../packages/plugin-sdk/src/client-modules.ts';
import type { ClientModuleFactory } from '../../../../packages/plugin-sdk/src/client-modules.ts';
import type { BrowserPluginHost } from '../../../../packages/plugin-sdk/src/context.ts';
import type { JsonValue } from '../../../../packages/client/src/types.ts';
import { invoke, clientScope } from '../client.ts';
import { models, registerScopeCleanup } from './models.ts';
import { canvas } from './canvas';

export const sharedClientModules = {
  react: React, 'react/jsx-runtime': jsxRuntime, 'react-dom': ReactDOM,
  'react-dom/client': ReactDOMClient, '@minerva/ui': { Button, Input, ...select, ...dropdown, ...sidebar, ...resizable, ...tooltip, ComposerMenuItem, ContextMenu, canvas },
};
export async function compileClientModule(source: string): Promise<ClientModuleFactory> {
  const url = URL.createObjectURL(new Blob([`export default function(require,module,exports){\n${source}\n}`], { type: 'text/javascript' }));
  try { return (await import(/* @vite-ignore */ url)).default; }
  finally { URL.revokeObjectURL(url); }
}
export function startExternalPlugins(host: BrowserPluginHost) {
  const loader = new ExternalClientModules({
    host, workspaceId: clientScope.workspace, modules: sharedClientModules, compile: compileClientModule,
    load: id => invoke('plugins.ui', { id }),
    rpc: (id, method, params) => invoke('plugins.call', { id, method, params: params as JsonValue }),
  });
  const sync = () => { void loader.sync(models.plugins.getSnapshot()); };
  const unsubscribe = models.plugins.subscribe(sync);
  registerScopeCleanup(() => { unsubscribe(); void loader.dispose().then(() => host.dispose()); });
  sync();
  return loader;
}
