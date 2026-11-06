export {invoke,configureClient} from '../../../packages/client/src/index.ts';
import {configureClient} from '../../../packages/client/src/index.ts';
import {createHttpClient} from '../../../packages/client/src/http.ts';
import type {MinervaClient} from '../../../packages/client/src/types.ts';
export const clientScope={server:globalThis.location?.origin||'tauri',principal:'local',workspace:'default'};
export let clientMode:'tauri'|'http'='http';
export async function initializeClient(client?:MinervaClient):Promise<void>{
 if(client){configureClient(client);return;}
 if('__TAURI_INTERNALS__' in globalThis){clientMode='tauri';const {createTauriClient}=await import('../../../packages/client/src/tauri.ts');configureClient(createTauriClient());}
 else {const workspaceId=new URL(globalThis.location.href).searchParams.get('workspaceId')??undefined;const http=await createHttpClient({workspaceId});Object.assign(clientScope,{principal:http.bootstrap.principal.id,workspace:http.bootstrap.workspace.id});configureClient(http);}
}
