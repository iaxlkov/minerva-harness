import {invoke, clientScope,clientMode} from '../client.ts';
import {SessionsModel} from '../../../../packages/client-models/src/sessions.ts';
import {ProvidersModel} from '../../../../packages/client-models/src/providers.ts';
import {PluginsModel} from '../../../../packages/client-models/src/plugins.ts';
import {RunsModel} from '../../../../packages/client-models/src/runs.ts';
import {Preferences} from '../../../../packages/client-models/src/preferences.ts';
export const models={sessions:new SessionsModel(invoke),providers:new ProvidersModel(invoke),plugins:new PluginsModel(invoke),runs:new RunsModel()};
export const preferences=()=>new Preferences(localStorage,clientScope);
/** Call before replacing credentials/workspace. Invalidates outstanding reads and subscriptions. */
const scopeCleanups=new Set<()=>void>();
export function registerScopeCleanup(cleanup:()=>void){scopeCleanups.add(cleanup);return ()=>{scopeCleanups.delete(cleanup);};}
export function clearClientState(logout=false){for(const cleanup of scopeCleanups)cleanup();scopeCleanups.clear();for(const model of Object.values(models))model.dispose();if(logout)preferences().clear();}

export function replaceClientScope(scope:typeof clientScope,logout=false){
 if(clientMode!=='http'||scope.server!==clientScope.server||scope.principal!==clientScope.principal)throw Error('Connection identity must be changed through authenticated login');
 const target=new URL(globalThis.location.href);target.searchParams.set('workspaceId',scope.workspace);
 clearClientState(logout);
 // Reboot client credentials, plugin scope and React state in a new document.
 // Only authenticated bootstrap may install the requested workspace identity.
 globalThis.location.assign(target.toString());
}
