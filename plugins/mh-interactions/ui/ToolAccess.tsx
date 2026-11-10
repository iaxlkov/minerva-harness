import {useEffect,useState} from 'react';
import {useAuiState} from '@assistant-ui/react';
import type {PendingInteraction} from '../../../packages/client/src/types';
import {invoke} from '../../../apps/web/src/client';
import {pollSession} from '../../../apps/web/src/harness/poll';
import {ToolApproval} from '../../../apps/web/src/components/assistant-ui/elements/tool-approval';

export function ToolAccess({sessionId,callId}:{sessionId:string;callId:string}){
 const runId=useAuiState(state=>state.message.id);
 const [pending,setPending]=useState<PendingInteraction>();
 useEffect(()=>pollSession(()=>invoke('interactions.list',{sessionId}),items=>setPending(items.find(item=>item.kind==='approval'&&item.sessionId===sessionId&&item.runId===runId&&item.callId===callId)),()=>setPending(undefined)),[sessionId,runId,callId]);
 if(!pending)return null;
 return <ToolApproval key={pending.id} prompt={pending.prompt} onAnswer={async approved=>{const accepted=await invoke('interactions.answer',{sessionId,id:pending.id,decision:approved?'allow':'deny',answer:null});if(!accepted)throw Error('Запрос истёк');setPending(undefined);}}/>;
}
