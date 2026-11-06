import {invoke} from './index.ts';
import type {SessionInfo} from './types.ts';
const valid:Promise<SessionInfo> = invoke('session_get',{id:'s'});
void valid;
// @ts-expect-error registered endpoint requires a string ID
invoke('session_get',{id:12});
// @ts-expect-error nonempty endpoint parameters cannot be omitted
invoke('session_get');
// @ts-expect-error method must be registered
invoke('missing_endpoint',{});
// @ts-expect-error result follows the endpoint, not caller-selected type
const wrong:Promise<string> = invoke('session_list');
void wrong;
invoke('session_list');
