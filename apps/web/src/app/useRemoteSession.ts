import {useEffect,useMemo,useSyncExternalStore} from 'react';
import {SessionModel} from '../../../../packages/client-models/src/session.ts';
import {createSessionStream} from '../../../../packages/client/src/streams.ts';
import {clientMode,clientScope} from '../client.ts';
import {registerScopeCleanup} from './models.ts';

export function useRemoteSession(sessionId:string){
 const model=useMemo(()=>new SessionModel(sessionId,createSessionStream({workspaceId:clientScope.workspace})),[sessionId,clientScope.server,clientScope.principal,clientScope.workspace]);
 useEffect(()=>{const unregister=registerScopeCleanup(model.dispose);if(clientMode==='http')void model.follow();return ()=>{unregister();model.dispose();};},[model]);
 return useSyncExternalStore(model.subscribe,model.getSnapshot);
}
