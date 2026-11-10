import {Store} from '../../../packages/client-models/src/store';
import type {WorkspaceInfo} from '../../../packages/client/src/types';
export const workspaceActions=new Store<{workspace:WorkspaceInfo;action:'rename'|'remove'}|null>(null);
