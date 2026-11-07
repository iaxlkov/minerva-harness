import React from 'react';
import {createRoot} from 'react-dom/client';
import {initializeClient,clientScope} from '../client.ts';
import {App} from './App';
import {createBrowserComposition,PluginRoot,workspaceScope} from './plugins';
import {startExternalPlugins} from './externalPlugins.ts';
void initializeClient().then(async () => {
 workspaceScope.workspaceId=clientScope.workspace;
 const host=await createBrowserComposition();
 const externalPlugins=startExternalPlugins(host);
 createRoot(document.getElementById('root')!).render(<React.StrictMode><PluginRoot host={host} externalPlugins={externalPlugins}><App/></PluginRoot></React.StrictMode>);
}).catch(error => {
 const root=document.getElementById('root')!;root.setAttribute('role','alert');root.textContent=`Unable to initialize Minerva: ${String(error)}`;
});
